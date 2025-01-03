mod git_ops;
mod gpt_ops;
mod tui;
use crate::git_ops::*;
use crate::gpt_ops::*;
use crate::tui::*;
use ratatui::{
    backend::{Backend, CrosstermBackend},
    crossterm::{
        event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    },
    style::Color,
    Terminal,
};
use std::{io, process::Command};
use tokio::time::{Duration, Instant};

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

    if let Err(e) = app_result {
        eprintln!("ERROR in execution: {}", e);
        // Print logs and errors after terminal is restored
        for (log_level, log_message) in &app.logs {
            println!("{}: {}", log_level, log_message);
        }
        for error in &app.errors {
            eprintln!("ERROR: {}", error);
        }
    }

    Ok(())
}

async fn run<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App<'_>,
    tick_rate: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut last_tick = Instant::now();

    // Start UI loop immediately to show initialization progress
    let ui_update = tokio::spawn({
        let mut last_tick = Instant::now();
        async move {
            loop {
                tokio::time::sleep(tick_rate).await;
                if last_tick.elapsed() >= tick_rate {
                    last_tick = Instant::now();
                }
            }
        }
    });

    // Initial UI render
    terminal.draw(|f| ui(f, app))?;

    let api_key = match keyring::Entry::new("gh-autopr", "openai_key") {
        Ok(entry) => match entry.get_password() {
            Ok(key) => {
                app.add_log("INFO", "Found OpenAI key in keyring");
                key
            }
            Err(_) => match std::env::var("OPENAI_KEY") {
                Ok(key) => {
                    app.add_log(
                        "INFO",
                        "Found OpenAI key in environment, storing in keyring",
                    );
                    if let Err(e) = entry.set_password(&key) {
                        app.add_error(format!("Failed to store key in keyring: {}", e));
                    }
                    key
                }
                Err(e) => {
                    app.add_error("OpenAI key not found in keyring or environment");
                    terminal.draw(|f| ui(f, app))?;
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    return Err(e.into());
                }
            },
        },
        Err(e) => {
            app.add_error("Failed to access keyring");
            app.switch_to_tab(1);
            terminal.draw(|f| ui(f, app))?;
            tokio::time::sleep(Duration::from_secs(2)).await;
            return Err(e.into());
        }
    };
    std::env::set_var("OPENAI_KEY", api_key);

    // Initialize OpenAI and GitHub logic
    app.add_log("INFO", "Initializing...");
    app.update_progress(0.1);
    terminal.draw(|f| ui(f, app))?;

    git_ensure_in_repo(app)?;
    terminal.draw(|f| ui(f, app))?;

    git_cd_to_repo_root(app)?;
    terminal.draw(|f| ui(f, app))?;

    let main_branch = git_main_branch(app).unwrap_or_else(|_| "main".to_string());
    terminal.draw(|f| ui(f, app))?;

    let mut current_branch = git_current_branch(app)?;
    let original_branch = current_branch.clone();
    terminal.draw(|f| ui(f, app))?;

    git_ensure_not_detached_head(terminal, app, &current_branch)?;
    terminal.draw(|f| ui(f, app))?;

    git_fetch_main(app, &current_branch, &main_branch)?;
    terminal.draw(|f| ui(f, app))?;

    app.add_log(
        "INFO",
        format!("Main branch: {main_branch}, Current branch: {current_branch}"),
    );
    app.update_progress(0.3);
    terminal.draw(|f| ui(f, app))?;

    let diff_uncommitted = git_diff_uncommitted(app)?;
    terminal.draw(|f| ui(f, app))?;

    if !diff_uncommitted.is_empty() {
        app.update_details(diff_uncommitted.clone());
        terminal.draw(|f| ui(f, app))?;

        app.add_log("INFO", "Generating branch name and commit description...");
        terminal.draw(|f| ui(f, app))?;

        let (branch_name, commit_title, commit_details) =
            gpt_generate_branch_name_and_commit_description(app, diff_uncommitted).await?;
        terminal.draw(|f| ui(f, app))?;

        if current_branch == main_branch {
            let output = Command::new("git")
                .args(["checkout", "-b", &branch_name])
                .output()?;
            current_branch = branch_name;
            app.add_log("INFO", String::from_utf8_lossy(&output.stdout).to_string());
            if !output.status.success() {
                app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
            }
            terminal.draw(|f| ui(f, app))?;
        }
        git_stage_and_commit(app, &commit_title, &commit_details)?;
        terminal.draw(|f| ui(f, app))?;
    } else if current_branch == main_branch {
        app.add_log("INFO", "No changes to commit.");
        terminal.draw(|f| ui(f, app))?;
        render_message(terminal, "Info", "No changes to commit.", Color::Cyan)?;
        return Ok(());
    }

    let diff_between_branches = match git_diff_between_branches(app, &main_branch, &current_branch)
    {
        Ok(diff) => diff,
        Err(err) => {
            app.add_error(err.to_string());
            app.switch_to_tab(1);
            terminal.draw(|f| ui(f, app))?;
            tokio::time::sleep(Duration::from_secs(2)).await;
            app.should_quit = true;
            return Err(err);
        }
    };
    terminal.draw(|f| ui(f, app))?;

    if diff_between_branches.is_empty() {
        app.add_log("INFO", "No changes between the branches.");
        terminal.draw(|f| ui(f, app))?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        app.should_quit = true;
        return Ok(());
    }

    app.add_log("INFO", "Generating PR details using AI...");
    app.update_progress(0.5);
    terminal.draw(|f| ui(f, app))?;

    let (_, commit_title, commit_details) =
        gpt_generate_branch_name_and_commit_description(app, diff_between_branches).await?;
    app.add_log("INFO", format!("Commit title: {commit_title}"));
    app.add_log(
        "INFO",
        format!(
            "Commit details: {}",
            commit_details.clone().unwrap_or_default()
        ),
    );
    terminal.draw(|f| ui(f, app))?;

    git_push_branch(app, &current_branch)?;
    terminal.draw(|f| ui(f, app))?;

    app.add_log("INFO", format!("Commit title: {commit_title}"));
    app.add_log("INFO", "Creating pull request...");
    app.update_progress(0.8);
    terminal.draw(|f| ui(f, app))?;

    let _ = create_or_update_pull_request(app, &commit_title, &commit_details.unwrap_or_default());
    terminal.draw(|f| ui(f, app))?;

    app.add_log("SUCCESS", "Pull request created successfully.");
    app.update_progress(1.0);
    terminal.draw(|f| ui(f, app))?;

    git_checkout_branch(app, &original_branch)?;
    terminal.draw(|f| ui(f, app))?;

    git_pull_branch(app, &original_branch)?;
    terminal.draw(|f| ui(f, app))?;

    // Cancel the UI progress-update task
    ui_update.abort();

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
