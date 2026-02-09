mod git_ops;
mod git_temp_worktree;
mod github_ops;
mod gpt_ops;
mod tui;
use crate::git_ops::*;
use crate::git_temp_worktree::*;
use crate::github_ops::*;
use crate::gpt_ops::*;
use crate::tui::*;
use clap::Parser;
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

    /// Prune local branches that have been merged
    #[arg(long, visible_aliases = ["prune", "cleanup"])]
    prune_branches: bool,
}

/// Configuration passed from CLI args to the run function
#[derive(Debug, Clone)]
struct RunConfig {
    update_pr: bool,
    ready: bool,
    what: Option<String>,
    why: Option<String>,
    bigger_picture: Option<String>,
}

/// Branch information gathered before entering temp worktree
struct BranchInfo {
    main_branch: String,
    current_branch: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Handle branch pruning early - no TUI needed
    if args.prune_branches {
        return run_prune_branches();
    }

    // Initialize the terminal for PR creation mode
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut app = App::new("GitHub PR Auto-Submit");
    let tick_rate = Duration::from_millis(250);

    let config = RunConfig {
        update_pr: args.update_pr,
        ready: args.ready,
        what: args.what,
        why: args.why,
        bigger_picture: args.bigger_picture,
    };

    // Do git operations that need original worktree BEFORE entering temp worktree
    let branch_info = match pre_worktree_setup(&mut terminal, &mut app, tick_rate).await {
        Ok(info) => info,
        Err(e) => {
            restore_terminal(&mut terminal)?;
            eprintln!("ERROR in pre-worktree setup: {}", e);
            return Err(e);
        }
    };

    // All subsequent Git commands act inside the isolated worktree
    let temp_worktree = TempWorktree::enter()?;

    let app_result = run(
        &mut terminal,
        &mut app,
        tick_rate,
        config,
        branch_info,
        temp_worktree,
    )
    .await;

    restore_terminal(&mut terminal)?;

    if let Err(ref e) = app_result {
        eprintln!("ERROR in execution: {}", e);
    }

    // Print logs after terminal is restored
    for (log_level, log_message) in &app.logs {
        println!("{}: {}", log_level, log_message);
    }

    app_result.map(|_| ())
}

fn restore_terminal<B: Backend + std::io::Write>(
    terminal: &mut Terminal<B>,
) -> Result<(), Box<dyn std::error::Error>> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn run_prune_branches() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::new("GitHub PR Auto-Submit");

    git_ensure_in_repo(&mut app)?;
    git_cd_to_repo_root(&mut app)?;

    let result = prune_merged_branches(&mut app);

    for (log_level, log_message) in &app.logs {
        println!("{}: {}", log_level, log_message);
    }

    match result {
        Ok(_) => {
            println!("Branch pruning completed successfully.");
            Ok(())
        }
        Err(e) => {
            eprintln!("ERROR in branch pruning: {}", e);
            Err(e)
        }
    }
}

async fn pre_worktree_setup<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App<'_>,
    tick_rate: Duration,
) -> Result<BranchInfo, Box<dyn std::error::Error>> {
    let mut last_tick = Instant::now();

    app.add_log("INFO", "Checking git repository...");
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;
    git_ensure_in_repo(app)?;

    app.add_log("INFO", "Navigating to repository root...");
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;
    git_cd_to_repo_root(app)?;

    let main_branch = git_main_branch(app).unwrap_or_else(|_| "main".to_string());
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    let current_branch = git_current_branch(app)?;
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    git_ensure_not_detached_head(terminal, app, &current_branch)?;
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    app.add_log("INFO", "Fetching latest changes...");
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;
    git_fetch_main(app, &current_branch, &main_branch)?;

    app.add_log("INFO", "Pre-worktree setup complete");
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    Ok(BranchInfo {
        main_branch,
        current_branch,
    })
}

async fn run<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App<'_>,
    tick_rate: Duration,
    config: RunConfig,
    branch_info: BranchInfo,
    temp_worktree: TempWorktree,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut last_tick = Instant::now();
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    // Get OpenAI API key
    let api_key = get_openai_key(app, terminal).await?;
    std::env::set_var("OPENAI_KEY", api_key);

    app.add_log("INFO", "Working in temp worktree...");
    app.update_progress(0.1);
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    // Use branch info from pre_worktree_setup (no need to re-detect)
    let main_branch = &branch_info.main_branch;
    let mut current_branch = branch_info.current_branch.clone();
    let is_on_main = current_branch == *main_branch;
    let base_branch = discover_parent_branch(app, main_branch, &current_branch)?;

    app.add_log(
        "INFO",
        format!(
            "Main: {}, Current: {}, Base: {}",
            main_branch, current_branch, base_branch
        ),
    );
    app.update_progress(0.2);
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    // Get uncommitted changes
    let diff_uncommitted = git_diff_uncommitted(app, &current_branch)?;
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    // Fetch GitHub issues once
    app.add_log("INFO", "Fetching GitHub issues...");
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;
    let issues_json = github_list_issues(app)?;

    // Track GPT response for reuse (avoid calling twice for fresh branches)
    let mut cached_gpt_response: Option<(String, Option<String>)> = None;

    if diff_uncommitted.is_empty() {
        if is_on_main {
            app.add_log("INFO", "No changes to commit.");
            render_message(terminal, "Info", "No changes to commit.", Color::Cyan)?;
            app.update_progress(1.0);
            terminal.draw(|f| ui(f, app))?;
            run_event_loop(terminal, app, tick_rate, &mut last_tick)?;
            return Ok(());
        }
    } else {
        app.update_details(diff_uncommitted.clone());
        refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

        app.add_log("INFO", "Generating branch name and commit message...");
        refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

        let (generated_branch_name, commit_title, commit_details) =
            gpt_generate_branch_name_and_commit_description(
                app,
                diff_uncommitted,
                Some(issues_json.clone()),
                config.what.clone(),
                config.why.clone(),
                config.bigger_picture.clone(),
            )
            .await?;
        refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

        // Create new branch if on main or creating new PR
        let creating_new_branch = is_on_main || !config.update_pr;
        if creating_new_branch {
            git_checkout_new_branch(app, &generated_branch_name, &current_branch, false)?;
            app.add_log("INFO", format!("Created branch: {}", generated_branch_name));
            current_branch = generated_branch_name;
            // Cache response - branch diff will be same as uncommitted diff
            cached_gpt_response = Some((commit_title.clone(), commit_details.clone()));
            terminal.draw(|f| ui(f, app))?;
        }

        refresh_ui(terminal, app, tick_rate, &mut last_tick)?;
        git_stage_and_commit(app, &commit_title, &commit_details)?;
        refresh_ui(terminal, app, tick_rate, &mut last_tick)?;
    }

    // Get diff between current branch and base
    let diff_between_branches = git_diff_between_branches(app, &base_branch, &current_branch)?;
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    if diff_between_branches.is_empty() {
        app.add_log("INFO", "No changes between branches.");
        terminal.draw(|f| ui(f, app))?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        return Ok(());
    }

    // Get PR title/body (reuse cached or generate new)
    let (pr_title, pr_body) = match cached_gpt_response {
        Some((title, details)) => {
            app.add_log("INFO", "Reusing generated content for PR...");
            app.update_progress(0.5);
            refresh_ui(terminal, app, tick_rate, &mut last_tick)?;
            (title, details)
        }
        None => {
            app.add_log("INFO", "Generating PR details...");
            app.update_progress(0.5);
            refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

            let (_, title, details) = gpt_generate_branch_name_and_commit_description(
                app,
                diff_between_branches,
                Some(issues_json),
                config.what,
                config.why,
                config.bigger_picture,
            )
            .await?;
            (title, details)
        }
    };

    app.add_log("INFO", format!("PR title: {}", pr_title));
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    // Push branch (may rename if remote conflict)
    current_branch = git_push_branch(app, &current_branch)?;
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    // Create or update PR
    app.add_log(
        "INFO",
        if config.update_pr {
            "Updating PR..."
        } else {
            "Creating PR..."
        },
    );
    app.update_progress(0.8);
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    create_or_update_pull_request(
        app,
        &pr_title,
        &pr_body.unwrap_or_default(),
        config.update_pr,
        config.ready,
        &base_branch,
        &current_branch,
    )?;

    app.add_log("SUCCESS", "Pull request created/updated successfully!");
    app.update_progress(1.0);
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    // Cleanup: update original worktree to PR branch
    let pr_branch = current_branch.clone();
    let orig_root = temp_worktree.original_root().clone();
    let had_staged = temp_worktree.had_staged_changes();

    app.add_log("INFO", "Switching original worktree to PR branch...");
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    // Wait for user before cleanup
    run_event_loop(terminal, app, tick_rate, &mut last_tick)?;

    // Drop temp worktree, then update original
    std::mem::drop(temp_worktree);
    update_original_worktree_to_pr_branch(app, &pr_branch, &orig_root, had_staged)?;

    Ok(())
}

async fn get_openai_key<B: Backend>(
    app: &mut App<'_>,
    terminal: &mut Terminal<B>,
) -> Result<String, Box<dyn std::error::Error>> {
    // Try keyring first
    if let Ok(entry) = keyring::Entry::new("gh-autopr", "openai_key") {
        if let Ok(key) = entry.get_password() {
            app.add_log("INFO", "Found OpenAI key in keyring");
            return Ok(key);
        }

        // Try environment variable and store in keyring
        if let Ok(key) = std::env::var("OPENAI_KEY") {
            app.add_log(
                "INFO",
                "Found OpenAI key in environment, storing in keyring",
            );
            let _ = entry.set_password(&key);
            return Ok(key);
        }
    }

    // Try environment variable without keyring
    if let Ok(key) = std::env::var("OPENAI_KEY") {
        app.add_log("INFO", "Found OpenAI key in environment");
        return Ok(key);
    }

    app.add_error("OpenAI key not found in keyring or environment");
    app.switch_to_tab(1);
    terminal.draw(|f| ui(f, app))?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    Err("OpenAI key not found".into())
}

fn run_event_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App<'_>,
    tick_rate: Duration,
    last_tick: &mut Instant,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        refresh_ui(terminal, app, tick_rate, last_tick)?;
        if app.should_quit {
            return Ok(());
        }
    }
}

/// Draws the UI and checks for user input events.
/// This is the main UI refresh function that should be called after state changes.
fn refresh_ui<B: Backend>(
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

    Ok(())
}
