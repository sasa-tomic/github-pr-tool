use log::{error, info};
use openai::{
    chat::{ChatCompletion, ChatCompletionMessage, ChatCompletionMessageRole},
    Credentials,
};
use ratatui::{
    backend::CrosstermBackend,
    crossterm::terminal::{disable_raw_mode, enable_raw_mode},
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Text},
    widgets::{Block, Borders, Gauge, Paragraph},
    Terminal,
};
use std::io;
use std::process::Command;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    if std::env::var("OPENAI_KEY").is_err() {
        render_message(
            &mut terminal,
            "Error",
            "Environment variable OPENAI_KEY is not set.",
            Color::Red,
        )?;
        std::process::exit(1);
    }

    git_ensure_in_repo()?;
    git_cd_to_repo_root()?;

    let main_branch = git_main_branch().unwrap_or_else(|_| "main".to_string());

    if main_branch.is_empty() {
        render_message(
            &mut terminal,
            "Error",
            "Unable to determine the upstream main branch.",
            Color::Red,
        )?;
        std::process::exit(1);
    }

    let mut current_branch = git_current_branch()?;
    git_ensure_not_detached_head(&mut terminal, &current_branch)?;

    git_fetch_main(&current_branch, &main_branch)?;

    render_message(
        &mut terminal,
        "Info",
        &format!(
            "Main branch: {}, current branch: {}",
            main_branch, current_branch
        ),
        Color::Cyan,
    )?;

    let diff_uncommitted = git_diff_uncommitted()?;

    if !diff_uncommitted.is_empty() {
        render_progress(&mut terminal, "Staging and committing changes...", 0.5)?;

        let (branch_name, commit_title, commit_details) =
            gpt_generate_branch_name_and_commit_description(diff_uncommitted).await?;

        if current_branch == main_branch {
            Command::new("git")
                .args(["checkout", "-b", &branch_name])
                .status()?;
        }

        git_stage_and_commit(&commit_title, &commit_details)?;
        current_branch = branch_name;
    } else if current_branch == main_branch {
        render_message(&mut terminal, "Info", "No changes to commit.", Color::Cyan)?;
        std::process::exit(0);
    }

    let diff_between_branches = git_diff_between_branches(&main_branch, &current_branch)?;
    if diff_between_branches.is_empty() {
        render_message(
            &mut terminal,
            "Info",
            "No changes between the branches.",
            Color::Cyan,
        )?;
        std::process::exit(0);
    }

    render_progress(&mut terminal, "Creating a pull request...", 0.8)?;

    let (_, commit_title, commit_details) =
        gpt_generate_branch_name_and_commit_description(diff_between_branches).await?;
    git_push_branch(&current_branch)?;
    gh_pr_create(&commit_title, &commit_details.unwrap_or_default())?;

    render_message(
        &mut terminal,
        "Success",
        "Pull request created successfully.",
        Color::Green,
    )?;

    disable_raw_mode()?;
    Ok(())
}

fn render_message(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
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

fn render_progress(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    message: &str,
    progress: f64,
) -> Result<(), io::Error> {
    terminal.draw(|f| {
        let area = f.size();
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

fn git_ensure_in_repo() -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()?;
    if !output.status.success() {
        error!("Not in a git repository.");
        std::process::exit(1);
    }
    Ok(())
}

fn git_ensure_not_detached_head(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
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

fn git_main_branch() -> Result<String, Box<dyn std::error::Error>> {
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

    Ok(String::from_utf8(main_branch_output.stdout)?
        .trim()
        .trim_start_matches("origin/")
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

fn gh_pr_create(title: &str, body: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Create a GitHub PR, now that we have a branch and a commit locally
    Command::new("gh")
        .args(["pr", "create", "--title", title, "--body", body])
        .status()?;
    Ok(())
}

async fn gpt_generate_branch_name_and_commit_description(
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

    info!("chat_response: {}", chat_response);
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
