mod git_ops;
mod gpt_ops;
mod tui;
use crate::git_ops::*;
use crate::gpt_ops::*;
use crate::tui::*;
use clap::Parser;

#[derive(Parser)]
#[command(
    author,
    version,
    about = "gh-autopr: Automate GitHub Pull Request creation.",
    long_about = "gh-autopr is a command-line tool that automates the process of creating GitHub Pull Requests. It analyzes your uncommitted changes, generates a branch name, commit message, and PR description using AI, and then pushes the changes and creates the PR."
)]
struct Args {
    /// Update an existing PR instead of creating a new one
    #[arg(long, visible_aliases = ["update-existing", "update"])]
    update_pr: bool,

    /// Create PR as ready for review instead of draft
    #[arg(long)]
    ready: bool,

    /// What changes are included in this PR?
    #[arg(long)]
    what: Option<String>,

    /// Why are these changes necessary?
    #[arg(long)]
    why: Option<String>,

    /// How do these changes fit into the bigger picture?
    #[arg(long, visible_aliases = ["bigger-picture", "biggerpicture", "context", "overview"])]
    bigger_picture: Option<String>,
}
use ratatui::{
    backend::{Backend, CrosstermBackend},
    crossterm::{
        event::{
            self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
            KeyModifiers,
        },
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    },
    style::Color,
    Terminal,
};
use tokio::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command line arguments
    let args = Args::parse();

    // All subsequent Git commands act inside the isolated worktree, that is automatically cleaned up.
    let _tw = TempWorktree::enter()?;

    // Initialize the terminal AFTER setting up signal handling
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut app = App::new("GitHub PR Auto-Submit");
    let tick_rate = Duration::from_millis(250);
    let app_result = run(
        &mut terminal,
        &mut app,
        tick_rate,
        args.update_pr,
        args.ready,
        args.what,
        args.why,
        args.bigger_picture,
    )
    .await;

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
    }
    // Print logs and errors after terminal is restored
    for (log_level, log_message) in &app.logs {
        println!("{}: {}", log_level, log_message);
    }

    Ok(())
}

async fn run<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App<'_>,
    tick_rate: Duration,
    update_pr: bool,
    ready: bool,
    what: Option<String>,
    why: Option<String>,
    bigger_picture: Option<String>,
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
    check_events(terminal, app, tick_rate, &mut last_tick)?;

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
    check_events(terminal, app, tick_rate, &mut last_tick)?;

    git_ensure_in_repo(app)?;
    terminal.draw(|f| ui(f, app))?;
    check_events(terminal, app, tick_rate, &mut last_tick)?;

    git_cd_to_repo_root(app)?;
    terminal.draw(|f| ui(f, app))?;
    check_events(terminal, app, tick_rate, &mut last_tick)?;

    let main_branch = git_main_branch(app).unwrap_or_else(|_| "main".to_string());
    terminal.draw(|f| ui(f, app))?;
    check_events(terminal, app, tick_rate, &mut last_tick)?;

    let mut current_branch = git_current_branch(app)?;
    let is_current_branch_main = current_branch == main_branch;
    // If on main branch ==> current_branch_merge_base is None
    // If not on main branch ==> current_branch_merge_base is the merge base with main
    let current_branch_merge_base = discover_parent_branch(app, &main_branch, &current_branch)?;

    let original_branch = current_branch.clone();
    terminal.draw(|f| ui(f, app))?;

    git_ensure_not_detached_head(terminal, app, &current_branch)?;
    terminal.draw(|f| ui(f, app))?;
    check_events(terminal, app, tick_rate, &mut last_tick)?;

    git_fetch_main(app, &current_branch, &main_branch)?;
    terminal.draw(|f| ui(f, app))?;
    check_events(terminal, app, tick_rate, &mut last_tick)?;

    app.add_log(
        "INFO",
        format!("Main branch: {main_branch}, Current branch: {current_branch}"),
    );
    app.update_progress(0.3);
    terminal.draw(|f| ui(f, app))?;
    check_events(terminal, app, tick_rate, &mut last_tick)?;

    let diff_uncommitted = git_diff_uncommitted(app, &current_branch)?;
    terminal.draw(|f| ui(f, app))?;
    check_events(terminal, app, tick_rate, &mut last_tick)?;

    if diff_uncommitted.is_empty() {
        if is_current_branch_main {
            app.add_log("INFO", "No changes to commit.");
            render_message(terminal, "Info", "No changes to commit.", Color::Cyan)?;
            app.update_progress(1.0);
            terminal.draw(|f| ui(f, app))?;
            ui_update.abort();
            run_event_loop(terminal, app, tick_rate, &mut last_tick)?;
            return Ok(());
        }
    } else {
        app.update_details(diff_uncommitted.clone());
        terminal.draw(|f| ui(f, app))?;
        check_events(terminal, app, tick_rate, &mut last_tick)?;

        app.add_log("INFO", "Fetching GitHub issues...");
        terminal.draw(|f| ui(f, app))?;
        check_events(terminal, app, tick_rate, &mut last_tick)?;

        let issues_json = git_list_issues(app)?;

        app.add_log("INFO", "Generating branch name and commit description...");
        terminal.draw(|f| ui(f, app))?;
        check_events(terminal, app, tick_rate, &mut last_tick)?;

        let (generated_branch_name, commit_title, commit_details) =
            gpt_generate_branch_name_and_commit_description(
                app,
                diff_uncommitted,
                Some(issues_json),
                what.clone(),
                why.clone(),
                bigger_picture.clone(),
            )
            .await?;
        terminal.draw(|f| ui(f, app))?;
        check_events(terminal, app, tick_rate, &mut last_tick)?;

        if is_current_branch_main || !update_pr {
            // Always create a new branch if a) on main or b) not asked to update an existing PR
            git_checkout_new_branch(app, &generated_branch_name, &current_branch, false)?;
            app.add_log(
                "INFO",
                format!("Created new branch: {generated_branch_name}"),
            );
            current_branch = generated_branch_name;
            terminal.draw(|f| ui(f, app))?;
        }

        check_events(terminal, app, tick_rate, &mut last_tick)?;
        git_stage_and_commit(app, &commit_title, &commit_details)?;
        terminal.draw(|f| ui(f, app))?;
        check_events(terminal, app, tick_rate, &mut last_tick)?;
    }

    let diff_between_branches =
        match git_diff_between_branches(app, &current_branch_merge_base, &current_branch) {
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
    check_events(terminal, app, tick_rate, &mut last_tick)?;

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
    check_events(terminal, app, tick_rate, &mut last_tick)?;

    app.add_log("INFO", "Fetching GitHub issues...");
    terminal.draw(|f| ui(f, app))?;
    check_events(terminal, app, tick_rate, &mut last_tick)?;

    let issues_json = git_list_issues(app)?;

    let (_, commit_title, commit_details) = gpt_generate_branch_name_and_commit_description(
        app,
        diff_between_branches,
        Some(issues_json),
        what,
        why,
        bigger_picture,
    )
    .await?;
    check_events(terminal, app, tick_rate, &mut last_tick)?;
    app.add_log("INFO", format!("Commit title: {commit_title}"));
    app.add_log(
        "INFO",
        format!(
            "Commit details: {}",
            commit_details.clone().unwrap_or_default()
        ),
    );
    terminal.draw(|f| ui(f, app))?;

    check_events(terminal, app, tick_rate, &mut last_tick)?;
    git_push_branch(app, &current_branch)?;
    terminal.draw(|f| ui(f, app))?;
    check_events(terminal, app, tick_rate, &mut last_tick)?;

    app.add_log("INFO", format!("Commit title: {commit_title}"));
    app.add_log("INFO", "Creating pull request...");
    app.update_progress(0.8);
    terminal.draw(|f| ui(f, app))?;
    check_events(terminal, app, tick_rate, &mut last_tick)?;

    if update_pr {
        app.add_log("INFO", "Updating existing pull request...");
    } else {
        app.add_log("INFO", "Creating new pull request...");
    }
    match create_or_update_pull_request(
        app,
        &commit_title,
        &commit_details.unwrap_or_default(),
        update_pr,
        ready,
        &current_branch_merge_base,
        &current_branch,
    ) {
        Ok(_) => {
            app.add_log("INFO", "Pull request created/updated successfully.");
        }
        Err(err) => {
            app.add_error(format!("Failed to create/update pull request: {}", err));
            app.switch_to_tab(1);
            terminal.draw(|f| ui(f, app))?;
            check_events(terminal, app, tick_rate, &mut last_tick)?;
            // Cancel the UI task to avoid concurrent draws in the error case.
            ui_update.abort();
            // Await user input so the user can see the error message before exiting.
            run_event_loop(terminal, app, tick_rate, &mut last_tick)?;
            return Err(err);
        }
    }
    terminal.draw(|f| ui(f, app))?;
    check_events(terminal, app, tick_rate, &mut last_tick)?;

    app.add_log("SUCCESS", "Pull request created successfully.");
    app.update_progress(1.0);
    terminal.draw(|f| ui(f, app))?;
    check_events(terminal, app, tick_rate, &mut last_tick)?;

    // Skip manual cleanup when in temp worktree - TempWorktree::Drop handles it automatically
    if !is_in_temp_worktree() {
        git_checkout_branch(app, &original_branch)?;
        terminal.draw(|f| ui(f, app))?;
        check_events(terminal, app, tick_rate, &mut last_tick)?;

        git_pull_branch(app, &original_branch)?;
        terminal.draw(|f| ui(f, app))?;
        check_events(terminal, app, tick_rate, &mut last_tick)?;

        git_stash_pop_autostash_if_exists(app)?;
        terminal.draw(|f| ui(f, app))?;
        check_events(terminal, app, tick_rate, &mut last_tick)?;
    } else {
        app.add_log("INFO", "Skipping manual cleanup - temp worktree will handle it automatically");
        terminal.draw(|f| ui(f, app))?;
        check_events(terminal, app, tick_rate, &mut last_tick)?;
    }

    // Cancel the UI progress-update task
    ui_update.abort();

    // Await user input before finishing so the UI remains visible.
    run_event_loop(terminal, app, tick_rate, &mut last_tick)?;
    Ok(())
}

fn run_event_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App<'_>,
    tick_rate: Duration,
    last_tick: &mut Instant,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        check_events(terminal, app, tick_rate, last_tick)?;
        if app.should_quit {
            return Ok(());
        }
    }
}

fn check_events<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App<'_>,
    tick_rate: Duration,
    last_tick: &mut Instant,
) -> Result<(), Box<dyn std::error::Error>> {
    terminal.draw(|f| ui(f, app))?;

    let timeout = tick_rate.saturating_sub(last_tick.elapsed());
    if event::poll(timeout)? {
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    KeyCode::Left => app.on_left(),
                    KeyCode::Right => app.on_right(),
                    KeyCode::Char('q') => app.should_quit = true,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        eprintln!("Ctrl+C detected. Reverting repository to original state...");
                        app.should_quit = true;
                        return Err("Interrupted by user".into());
                    }
                    _ => {}
                }
            }
        }
    }

    if last_tick.elapsed() >= tick_rate {
        *last_tick = Instant::now();
    }

    if app.should_quit {
        return Ok(());
    }
    Ok(())
}
