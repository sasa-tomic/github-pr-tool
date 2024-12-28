use openai::{
    chat::{ChatCompletion, ChatCompletionMessage, ChatCompletionMessageRole},
    Credentials,
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    crossterm::{
        event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    },
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Text},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Tabs},
    Terminal,
};
use std::{io, process::Command};
use tokio::time::{Duration, Instant};

// References:
// https://github.com/ratatui/ratatui/blob/main/examples/apps/demo/src/app.rs
// https://github.com/ratatui/ratatui/blob/main/examples/apps/demo/src/crossterm.rs

pub struct TabsState<'a> {
    pub titles: Vec<&'a str>,
    pub index: usize,
}

impl<'a> TabsState<'a> {
    pub fn new(titles: Vec<&'a str>) -> Self {
        Self { titles, index: 0 }
    }
    pub fn next(&mut self) {
        self.index = (self.index + 1) % self.titles.len();
    }
    pub fn previous(&mut self) {
        if self.index > 0 {
            self.index -= 1;
        } else {
            self.index = self.titles.len() - 1;
        }
    }
}

pub struct App<'a> {
    pub title: &'a str,
    pub should_quit: bool,
    pub tabs: TabsState<'a>,
    pub logs: Vec<(&'a str, String)>,
    pub progress: f64,
    pub details: String,
}

impl<'a> App<'a> {
    pub fn new(title: &'a str) -> Self {
        App {
            title,
            should_quit: false,
            tabs: TabsState::new(vec!["Logs", "Progress", "Details", "Status"]),
            logs: vec![],
            progress: 0.0,
            details: String::new(),
        }
    }

    pub fn on_up(&mut self) {}

    pub fn on_down(&mut self) {}

    pub fn on_right(&mut self) {
        self.tabs.next();
    }

    pub fn on_left(&mut self) {
        self.tabs.previous();
    }

    pub fn on_key(&mut self, c: char) {
        if c == 'q' {
            self.should_quit = true;
        }
    }

    pub fn update_progress(&mut self, value: f64) {
        self.progress = value;
    }

    pub fn add_log<S: ToString>(&mut self, level: &'a str, message: S) {
        self.logs.push((level, message.to_string()));
    }

    pub fn update_details(&mut self, details: String) {
        self.details = details;
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize the terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut app = App::new("GitHub PR Auto-Submit");
    let tick_rate = Duration::from_millis(250);
    let app_result = run(&mut terminal, &mut app, tick_rate).await;

    // restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = app_result {
        println!("{err:?}");
    }

    Ok(())
}

async fn run<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App<'_>,
    tick_rate: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut last_tick = Instant::now();
    if std::env::var("OPENAI_KEY").is_err() {
        app.add_log("ERROR", "Environment variable OPENAI_KEY is not set.");
        tokio::time::sleep(Duration::from_secs(2)).await;
        std::process::exit(1);
    }

    // Initialize OpenAI and GitHub logic
    app.add_log("INFO", "Initializing...");
    app.update_progress(0.1);

    git_ensure_in_repo(app)?;
    git_cd_to_repo_root()?;

    let main_branch = git_main_branch(app).unwrap_or_else(|_| "main".to_string());
    let mut current_branch = git_current_branch()?;
    git_ensure_not_detached_head(terminal, &current_branch)?;

    git_fetch_main(&current_branch, &main_branch)?;

    app.add_log(
        "INFO",
        format!("Main branch: {main_branch}, Current branch: {current_branch}"),
    );
    app.update_progress(0.3);

    let diff_uncommitted = git_diff_uncommitted()?;

    if !diff_uncommitted.is_empty() {
        app.update_details(diff_uncommitted.clone());
        render_progress_popup(terminal, "Staging and committing changes...", 0.5)?;

        let (branch_name, commit_title, commit_details) =
            gpt_generate_branch_name_and_commit_description(app, diff_uncommitted).await?;

        if current_branch == main_branch {
            Command::new("git")
                .args(["checkout", "-b", &branch_name])
                .status()?;
        }

        git_stage_and_commit(&commit_title, &commit_details)?;
        current_branch = branch_name;
    } else if current_branch == main_branch {
        render_message(terminal, "Info", "No changes to commit.", Color::Cyan)?;
        std::process::exit(0);
    }

    let diff_between_branches = git_diff_between_branches(&main_branch, &current_branch)?;
    if diff_between_branches.is_empty() {
        app.add_log("INFO", "No changes between the branches.");
        tokio::time::sleep(Duration::from_secs(2)).await;
        app.should_quit = true;
        std::process::exit(0);
    }

    render_progress_popup(terminal, "Creating a pull request...", 0.8)?;

    app.add_log("INFO", "Generating PR details using AI...");
    app.update_progress(0.5);
    let (_, commit_title, commit_details) =
        gpt_generate_branch_name_and_commit_description(app, diff_between_branches).await?;
    git_push_branch(&current_branch)?;

    app.add_log("INFO", format!("Commit title: {commit_title}"));
    app.add_log("INFO", "Creating pull request...");
    app.update_progress(0.8);

    create_pull_request(&commit_title, &commit_details.unwrap_or_default())?;

    app.add_log("Success", "Pull request created successfully.");
    app.update_progress(1.0);

    loop {
        terminal.draw(|f| ui(f, app))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Left => app.on_left(),
                        KeyCode::Right => app.on_right(),
                        KeyCode::Char('q') => app.should_quit = true,
                        _ => {}
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

fn ui(f: &mut ratatui::Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(f.area());

    let tabs = Tabs::new(
        app.tabs
            .titles
            .iter()
            .map(|t| Span::styled(*t, Style::default().fg(Color::Green)))
            .collect::<Vec<_>>(),
    )
    .block(Block::default().borders(Borders::ALL).title(app.title))
    .select(app.tabs.index)
    .highlight_style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    f.render_widget(tabs, chunks[0]);

    match app.tabs.index {
        0 => render_logs(f, app, chunks[2]),
        1 => render_progress(f, app, chunks[2]),
        2 => render_details(f, app, chunks[2]),
        3 => render_status(f, app, chunks[2]),
        _ => {}
    }
}

// This function displays progress (a gauge) in the main UI
fn render_progress(f: &mut ratatui::Frame, app: &App, area: ratatui::layout::Rect) {
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("Progress"))
        .gauge_style(
            Style::default()
                .fg(Color::Green)
                .bg(Color::Black)
                .add_modifier(Modifier::ITALIC),
        )
        .ratio(app.progress);
    f.render_widget(gauge, area);
}

fn render_details(f: &mut ratatui::Frame, app: &App, area: ratatui::layout::Rect) {
    let paragraph = Paragraph::new(Text::from(app.details.clone()))
        .block(Block::default().borders(Borders::ALL).title("Details"));
    f.render_widget(paragraph, area);
}

fn render_status(f: &mut ratatui::Frame, _app: &App, area: ratatui::layout::Rect) {
    let status_message = "All systems operational."; // Example status
    let paragraph = Paragraph::new(Text::from(status_message))
        .block(Block::default().borders(Borders::ALL).title("Status"));
    f.render_widget(paragraph, area);
}

fn render_logs(f: &mut ratatui::Frame, app: &App, area: ratatui::layout::Rect) {
    let logs: Vec<ListItem> = app
        .logs
        .iter()
        .map(|(level, message)| {
            let style = match *level {
                "INFO" => Style::default().fg(Color::Blue),
                "ERROR" => Style::default().fg(Color::Red),
                "SUCCESS" => Style::default().fg(Color::Green),
                "CRITICAL" => Style::default().fg(Color::Magenta),
                "WARNING" => Style::default().fg(Color::Yellow),
                _ => Style::default().fg(Color::Gray),
            };
            ListItem::new(Span::styled(message.clone(), style))
        })
        .collect();
    let logs_widget = List::new(logs).block(Block::default().borders(Borders::ALL).title("Logs"));
    f.render_widget(logs_widget, area);
}

// Renders a pop-up with the given message and progress ratio
fn render_progress_popup<B: Backend>(
    terminal: &mut Terminal<B>,
    message: &str,
    progress: f64,
) -> Result<(), io::Error> {
    terminal.draw(|f| {
        let area = f.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(80), Constraint::Percentage(20)].as_ref())
            .split(area);

        let block = Block::default().borders(Borders::ALL).title(Span::styled(
            "Progress",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        let paragraph = Paragraph::new(Text::from(message)).block(block);
        f.render_widget(paragraph, chunks[0]);

        let gauge = Gauge::default()
            .block(Block::default().borders(Borders::ALL).title("Progress"))
            .gauge_style(
                Style::default()
                    .fg(Color::Green)
                    .bg(Color::Black)
                    .add_modifier(Modifier::ITALIC),
            )
            .ratio(progress);
        f.render_widget(gauge, chunks[1]);
    })?;
    Ok(())
}

fn render_message<B: Backend>(
    terminal: &mut Terminal<B>,
    title: &str,
    message: &str,
    color: Color,
) -> Result<(), io::Error> {
    terminal.draw(|f| {
        let area = f.area();
        let block = Block::default().borders(Borders::ALL).title(Span::styled(
            title,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
        let paragraph = Paragraph::new(Text::from(message)).block(block);
        f.render_widget(paragraph, area);
    })?;
    Ok(())
}

fn git_ensure_in_repo(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()?;

    if !output.status.success() {
        app.add_log("ERROR", "Not in a git repository.");
        std::process::exit(1);
    }

    Ok(())
}

fn git_ensure_not_detached_head<B: Backend>(
    terminal: &mut Terminal<B>,
    branch_name: &String,
) -> Result<(), Box<dyn std::error::Error>> {
    if branch_name == "HEAD" {
        render_message(
            terminal,
            "Error",
            "Detached HEAD state detected. Please check out a branch.",
            Color::Red,
        )?;
        std::process::exit(1);
    }
    Ok(())
}

fn git_main_branch(app: &mut App) -> Result<String, Box<dyn std::error::Error>> {
    let mut main_branch_output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "origin/HEAD"])
        .output()?;

    if !main_branch_output.status.success() {
        Command::new("git")
            .args(["remote", "set-head", "origin", "--auto"])
            .status()?;

        main_branch_output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "origin/HEAD"])
            .output()?;
    }
    if !main_branch_output.status.success() {
        app.add_log(
            "ERROR",
            format!(
                "Failed to determine main branch: {}",
                String::from_utf8(main_branch_output.stderr)?
            ),
        );
        return Err("Failed to determine main branch.".into());
    }

    Ok(String::from_utf8(main_branch_output.stdout)?
        .trim()
        .trim_start_matches("origin/")
        .to_string())
}

fn git_cd_to_repo_root() -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if output.status.success() {
        let repo_root = String::from_utf8(output.stdout)?.trim().to_string();
        std::env::set_current_dir(repo_root)?;
    }
    Ok(())
}

fn git_diff_uncommitted() -> Result<String, Box<dyn std::error::Error>> {
    let diff_context = String::from_utf8(
        Command::new("git")
            .args(["diff", "--cached", "--", ".", ":!*.lock"])
            .output()?
            .stdout,
    )?
    .trim()
    .to_string();

    if diff_context.is_empty() {
        return Ok(String::from_utf8(
            Command::new("git")
                .args(["diff", "--", ".", ":!*.lock"])
                .output()?
                .stdout,
        )?
        .trim()
        .to_string());
    }

    Ok(diff_context)
}

fn git_diff_between_branches(
    main_branch: &String,
    current_branch: &String,
) -> Result<String, Box<dyn std::error::Error>> {
    Ok(String::from_utf8(
        Command::new("git")
            .args([
                "diff",
                &format!("{}...{}", main_branch, current_branch),
                "--",
                ":!*.lock",
            ])
            .output()?
            .stdout,
    )?
    .trim()
    .to_string())
}

fn git_current_branch() -> Result<String, std::io::Error> {
    Ok(String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()?
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string())
}

fn git_fetch_main(current_branch: &String, main_branch: &String) -> Result<(), std::io::Error> {
    if current_branch == main_branch {
        Command::new("git").args(["pull", "origin"]).status()?;
    } else {
        Command::new("git")
            .args([
                "fetch",
                "origin",
                format!("{}:{}", main_branch, main_branch).as_str(),
            ])
            .status()?;
    }

    Ok(())
}

fn git_stage_and_commit(
    commit_title: &str,
    commit_details: &Option<String>,
) -> Result<(), std::io::Error> {
    Command::new("git").args(["add", "."]).status()?;

    let mut commit_message = commit_title.trim().to_string();
    if let Some(details) = commit_details {
        commit_message.push_str(&format!("\n\n{}", details.trim()));
    }

    Command::new("git")
        .args(["commit", "-m", &commit_message])
        .status()?;

    Ok(())
}

fn git_push_branch(branch_name: &str) -> Result<(), std::io::Error> {
    Command::new("git")
        .args(["push", "origin", branch_name])
        .status()?;
    Ok(())
}

fn create_pull_request(title: &str, body: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Create a GitHub PR, now that we have a branch and a commit locally
    Command::new("gh")
        .args(["pr", "create", "--title", title, "--body", body])
        .status()?;
    Ok(())
}

async fn gpt_generate_branch_name_and_commit_description(
    app: &mut App<'_>,
    diff_context: String,
) -> Result<(String, String, Option<String>), Box<dyn std::error::Error>> {
    let credentials = Credentials::from_env();
    let messages = vec![
        ChatCompletionMessage {
            role: ChatCompletionMessageRole::System,
            content: Some(
                "You are a helpful assistant that helps to prepare GitHub PRs. You will provide output in JSON format with keys: 'branch_name', 'commit_title', and 'commit_details'. For a very small PR return 'commit_details' as null, otherwise humbly and politely in a well structured markdown format describe all changes in the PR. Do not describe the impact unless there is a breaking change. Follow the Conventional Commits specification for formatting PR descriptions.".to_string(),
            ),
            ..Default::default()
        },
        ChatCompletionMessage {
            role: ChatCompletionMessageRole::User,
            content: Some(format!(
                "Context:\n{}",
                diff_context
            )),
            ..Default::default()
        },
    ];
    let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());

    let chat_request = ChatCompletion::builder(&model, messages.clone())
        .credentials(credentials.clone())
        .create()
        .await?;
    let chat_response = chat_request
        .choices
        .first()
        .unwrap()
        .message
        .content
        .clone()
        .unwrap_or_default();

    let chat_response = chat_response
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .to_string();

    app.add_log("INFO", format!("chat_response: {}", chat_response));
    // Parse the JSON response
    let parsed_response: serde_json::Value = serde_json::from_str(&chat_response)?;
    let branch_name = parsed_response["branch_name"]
        .as_str()
        .unwrap_or("my-pr-branch")
        .to_string();
    let commit_title = parsed_response["commit_title"]
        .as_str()
        .unwrap_or("Generic commit title")
        .to_string();
    let commit_details = parsed_response["commit_details"]
        .as_str()
        .map(|s| s.to_string());

    Ok((branch_name, commit_title, commit_details))
}
