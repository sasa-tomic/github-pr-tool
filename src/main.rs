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

    // Handle branch pruning early - no TUI needed
    if args.prune_branches {
        let mut app = App::new("GitHub PR Auto-Submit");

        // Basic git checks without TUI
        git_ensure_in_repo(&mut app)?;
        git_cd_to_repo_root(&mut app)?;

        // Run pruning operation
        let prune_result = prune_merged_branches(&mut app);

        // Print logs
        for (log_level, log_message) in &app.logs {
            println!("{}: {}", log_level, log_message);
        }

        // Handle result
        match prune_result {
            Ok(_) => println!("Branch pruning completed successfully."),
            Err(e) => {
                eprintln!("ERROR in branch pruning: {}", e);
                return Err(e);
            }
        }
        return Ok(());
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
    let pre_worktree_result = pre_worktree_setup(&mut terminal, &mut app, tick_rate).await;

    // Handle any pre-worktree errors
    if let Err(e) = pre_worktree_result {
        // restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;
        eprintln!("ERROR in pre-worktree setup: {}", e);
        return Err(e);
    }

    // All subsequent Git commands act inside the isolated worktree, that is automatically cleaned up.
    let tw = TempWorktree::enter()?;

    let app_result = run(&mut terminal, &mut app, tick_rate, config, tw).await;

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

    // Clean up patch files older than 30 days to prevent accumulation
    let _ = cleanup_old_patches(&mut app, 30);

    Ok(())
}

#[derive(Debug, Clone)]
struct RunConfig {
    update_pr: bool,
    ready: bool,
    what: Option<String>,
    why: Option<String>,
    bigger_picture: Option<String>,
}

async fn pre_worktree_setup<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App<'_>,
    tick_rate: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut last_tick = Instant::now();

    // Check that we're in a git repo
    app.add_log("INFO", "Checking git repository...");
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;
    git_ensure_in_repo(app)?;

    // Navigate to repo root
    app.add_log("INFO", "Navigating to repository root...");
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;
    git_cd_to_repo_root(app)?;

    // Get branch information
    let main_branch = git_main_branch(app).unwrap_or_else(|_| "main".to_string());
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    let current_branch = git_current_branch(app)?;
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    // Ensure we're not in detached HEAD
    git_ensure_not_detached_head(terminal, app, &current_branch)?;
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    // Fetch main branch - THIS MUST HAPPEN BEFORE ENTERING TEMP WORKTREE
    app.add_log("INFO", "Fetching latest changes...");
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;
    git_fetch_main(app, &current_branch, &main_branch)?;

    app.add_log("INFO", "Pre-worktree setup completed successfully");
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    Ok(())
}

async fn run<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App<'_>,
    tick_rate: Duration,
    config: RunConfig,
    temp_worktree: TempWorktree,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut last_tick = Instant::now();

    // Initial UI render
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

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
    app.add_log("INFO", "Initializing in temp worktree...");
    app.update_progress(0.1);
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    // Re-detect branch information in temp worktree context
    let main_branch = git_main_branch(app).unwrap_or_else(|_| "main".to_string());
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    let mut current_branch = git_current_branch(app)?;
    let is_current_branch_main = current_branch == main_branch;
    // If on main branch ==> current_branch_merge_base is None
    // If not on main branch ==> current_branch_merge_base is the merge base with main
    let current_branch_merge_base = discover_parent_branch(app, &main_branch, &current_branch)?;

    let original_branch = current_branch.clone();
    terminal.draw(|f| ui(f, app))?;

    app.add_log(
        "INFO",
        format!("Main branch: {main_branch}, Current branch: {current_branch}"),
    );
    app.update_progress(0.3);
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    let diff_uncommitted = git_diff_uncommitted(app, &current_branch)?;
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    // Fetch GitHub issues once - will be reused for all GPT calls
    app.add_log("INFO", "Fetching GitHub issues...");
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;
    let issues_json = github_list_issues(app)?;

    // Track whether we created a fresh branch from uncommitted changes.
    // If so, we can reuse the GPT response for PR instead of calling again.
    let mut cached_gpt_response: Option<(String, Option<String>)> = None;

    if diff_uncommitted.is_empty() {
        if is_current_branch_main {
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

        app.add_log("INFO", "Generating branch name and commit description...");
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

        let created_fresh_branch = is_current_branch_main || !config.update_pr;
        if created_fresh_branch {
            // Creating a new branch - cache the GPT response to reuse for PR
            // since the branch diff will be essentially the same as uncommitted diff
            git_checkout_new_branch(app, &generated_branch_name, &current_branch, false)?;
            app.add_log(
                "INFO",
                format!("Created new branch: {generated_branch_name}"),
            );
            current_branch = generated_branch_name;
            cached_gpt_response = Some((commit_title.clone(), commit_details.clone()));
            terminal.draw(|f| ui(f, app))?;
        }

        refresh_ui(terminal, app, tick_rate, &mut last_tick)?;
        git_stage_and_commit(app, &commit_title, &commit_details)?;
        refresh_ui(terminal, app, tick_rate, &mut last_tick)?;
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
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    if diff_between_branches.is_empty() {
        app.add_log("INFO", "No changes between the branches.");
        terminal.draw(|f| ui(f, app))?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        app.should_quit = true;
        return Ok(());
    }

    // Reuse cached GPT response if we just created a fresh branch,
    // otherwise call GPT with the full branch diff (for existing branches with history)
    let (pr_title, pr_body) = if let Some((title, details)) = cached_gpt_response {
        app.add_log("INFO", "Reusing GPT response for PR (fresh branch)...");
        app.update_progress(0.5);
        refresh_ui(terminal, app, tick_rate, &mut last_tick)?;
        (title, details)
    } else {
        app.add_log("INFO", "Generating PR details using AI...");
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
    };
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;
    app.add_log("INFO", format!("PR title: {pr_title}"));
    app.add_log(
        "INFO",
        format!("PR body: {}", pr_body.clone().unwrap_or_default()),
    );
    terminal.draw(|f| ui(f, app))?;

    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;
    git_push_branch(app, &current_branch)?;
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    app.add_log("INFO", "Creating pull request...");
    app.update_progress(0.8);
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    if config.update_pr {
        app.add_log("INFO", "Updating existing pull request...");
    } else {
        app.add_log("INFO", "Creating new pull request...");
    }
    match create_or_update_pull_request(
        app,
        &pr_title,
        &pr_body.unwrap_or_default(),
        config.update_pr,
        config.ready,
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
            refresh_ui(terminal, app, tick_rate, &mut last_tick)?;
            // Await user input so the user can see the error message before exiting.
            run_event_loop(terminal, app, tick_rate, &mut last_tick)?;
            return Err(err);
        }
    }
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    app.add_log("SUCCESS", "Pull request created successfully.");
    app.update_progress(1.0);
    refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

    // Handle cleanup differently for temp worktree vs regular worktree
    if !is_in_temp_worktree() {
        // Regular worktree cleanup
        git_checkout_branch(app, &original_branch)?;
        refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

        git_pull_branch(app, &original_branch)?;
        refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

        git_stash_pop_autostash_if_exists(app)?;
        refresh_ui(terminal, app, tick_rate, &mut last_tick)?;
    } else {
        // Temp worktree cleanup - update original worktree to PR branch
        app.add_log(
            "INFO",
            "Preparing original worktree to switch to PR branch...",
        );
        refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

        // Store information for cleanup after temp worktree is dropped
        let pr_branch = current_branch.clone();
        let orig_branch = original_branch.clone();
        let orig_root = temp_worktree.original_root().clone();

        app.add_log(
            "INFO",
            "Original worktree will be switched to PR branch after cleanup",
        );
        refresh_ui(terminal, app, tick_rate, &mut last_tick)?;

        // Await user input before finishing so the UI remains visible.
        run_event_loop(terminal, app, tick_rate, &mut last_tick)?;

        // Drop temp worktree explicitly to clean it up
        std::mem::drop(temp_worktree);

        // Now update the original worktree to PR branch (after temp worktree is cleaned up)
        update_original_worktree_to_pr_branch(app, &pr_branch, &orig_branch, &orig_root)?;
    }

    // Await user input before finishing so the UI remains visible (if not already done)
    if !is_in_temp_worktree() {
        run_event_loop(terminal, app, tick_rate, &mut last_tick)?;
    }

    Ok(())
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
