use crate::tui::{render_message, App};
use ratatui::style::Color;
use ratatui::{backend::Backend, Terminal};
use std::process::Command;

const AUTOCOMMIT_BRANCH_NAME: &str = "gh-autopr-index-autocommit";
const AUTOSTASH_NAME: &str = "gh-autopr-index-autostash";

pub fn git_ensure_in_repo(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()?;

    if !output.status.success() {
        app.add_log("ERROR", "Not in a git repository.");
        std::process::exit(1);
    }

    Ok(())
}

pub fn git_ensure_not_detached_head<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    branch_name: &String,
) -> Result<(), Box<dyn std::error::Error>> {
    if branch_name == "HEAD" {
        app.add_log(
            "ERROR",
            "Detached HEAD state detected. Please check out a branch.",
        );
        render_message(
            terminal,
            "Error",
            "Detached HEAD state detected. Please check out a branch.",
            Color::Red,
        )?;
        return Err("Detached HEAD state detected".into());
    }
    Ok(())
}

pub fn git_cd_to_repo_root(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if output.status.success() {
        let repo_root = String::from_utf8(output.stdout)?.trim().to_string();
        std::env::set_current_dir(&repo_root)?;
        app.add_log(
            "INFO",
            format!("Changed directory to repo root: {}", repo_root),
        );
    } else {
        app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
    }
    Ok(())
}

pub fn git_diff_uncommitted(app: &mut App) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(["diff", "--cached", "--", ".", ":!*.lock"])
        .output()?;

    if !output.status.success() {
        app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
        return Err("Failed to get diff".into());
    }

    let diff_context = String::from_utf8(output.stdout)?.trim().to_string();

    if diff_context.is_empty() {
        let output = Command::new("git")
            .args(["diff", "--", ".", ":!*.lock"])
            .output()?;

        if !output.status.success() {
            app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
            return Err("Failed to get diff".into());
        }

        return Ok(String::from_utf8(output.stdout)?.trim().to_string());
    }

    Ok(diff_context)
}

pub fn git_diff_between_branches(
    app: &mut App,
    main_branch: &String,
    current_branch: &String,
) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args([
            "diff",
            &format!("{}...{}", main_branch, current_branch),
            "--",
            ".",
            ":!*.lock",
        ])
        .output()?;

    if !output.status.success() {
        app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
        return Err(format!(
            "Failed to get diff between branches: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

pub fn git_main_branch(app: &mut App) -> Result<String, Box<dyn std::error::Error>> {
    let mut main_branch_output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "origin/HEAD"])
        .output()?;

    if !main_branch_output.status.success() {
        app.add_log("INFO", "Setting origin HEAD automatically...");
        let output = Command::new("git")
            .args(["remote", "set-head", "origin", "--auto"])
            .output()?;

        if !output.status.success() {
            app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
            return Err("Failed to set origin HEAD".into());
        }

        main_branch_output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "origin/HEAD"])
            .output()?;

        if !main_branch_output.status.success() {
            app.add_error(String::from_utf8_lossy(&main_branch_output.stderr).to_string());
            return Err("Failed to determine main branch".into());
        }
    }

    let branch = String::from_utf8(main_branch_output.stdout)?
        .trim()
        .trim_start_matches("origin/")
        .to_string();
    app.add_log("INFO", format!("Determined main branch: {}", branch));
    Ok(branch)
}

pub fn git_current_branch(app: &mut App) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()?;

    if !output.status.success() {
        app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
        return Err("Failed to get current branch".into());
    }

    let branch = String::from_utf8(output.stdout)?.trim().to_string();
    app.add_log("INFO", format!("Current branch: {}", branch));
    Ok(branch)
}

pub fn git_fetch_main(
    app: &mut App,
    current_branch: &String,
    main_branch: &String,
) -> Result<(), Box<dyn std::error::Error>> {
    if current_branch == main_branch {
        let had_staged_changes = git_has_staged_changes()?;
        if had_staged_changes {
            app.add_log("INFO", "Staged changes detected, stashing in temp branch");
            git_checkout_new_branch(app, AUTOCOMMIT_BRANCH_NAME, true)?;
            git_commit_staged_changes(app, "Temporary commit for stashing changes", &None)?;
            // Stash all other changes
            Command::new("git")
                .args(["stash", "push", "-m", AUTOSTASH_NAME, "--include-untracked"])
                .output()?;
            // Return to main branch
            git_checkout_branch(app, main_branch)?;
        }
        let output = Command::new("git").args(["pull", "origin"]).output()?;
        if !output.status.success() {
            app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
            return Err("Failed to pull from origin".into());
        }
        app.add_log("INFO", "Pulled latest changes from origin");
        if had_staged_changes {
            // Add changes from AUTOCOMMIT_BRANCH_NAME to the index (staged): git cherry-pick AUTOCOMMIT_BRANCH_NAME~0
            let output = Command::new("git")
                .args(["cherry-pick", &format!("{}~0", AUTOCOMMIT_BRANCH_NAME)])
                .output()?;
            if !output.status.success() {
                app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
                return Err(format!(
                    "Failed to cherry-pick staged changes to the latest {}",
                    main_branch
                )
                .into());
            }
            app.add_log(
                "INFO",
                "Cherry-picked staged changes to the latest main branch",
            );
            // Reset the last commit: git reset --soft HEAD~1
            let output = Command::new("git")
                .args(["reset", "--soft", "HEAD~1"])
                .output()?;
            if !output.status.success() {
                app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
                return Err("Failed to reset the last commit".into());
            }
            app.add_log("INFO", "Reset the last commit");
            // Stage all changes: git add .
            let output = Command::new("git").args(["add", "."]).output()?;
            if !output.status.success() {
                app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
                return Err("Failed to stage all changes".into());
            }
        }
    } else {
        let output = Command::new("git")
            .args([
                "fetch",
                "origin",
                format!("{}:{}", main_branch, main_branch).as_str(),
            ])
            .output()?;
        if !output.status.success() {
            app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
            return Err("Failed to fetch main branch".into());
        }
        app.add_log("INFO", format!("Fetched latest {} branch", main_branch));
    }

    Ok(())
}

pub fn git_checkout_branch(
    app: &mut App,
    branch_name: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(["checkout", branch_name])
        .output()?;

    if !output.status.success() {
        app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
        return Err("Failed to checkout branch".into());
    }

    app.add_log("INFO", format!("Checked out branch: {}", branch_name));
    Ok(String::from_utf8_lossy(output.stdout.as_slice()).to_string())
}

pub fn git_checkout_new_branch(
    app: &mut App,
    branch_name: &str,
    force_reset: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut args = vec!["checkout"];
    if force_reset {
        args.push("-B");
    } else {
        let output = Command::new("git")
            .args(["rev-parse", "--verify", branch_name])
            .output()?;
        if output.status.success() {
            let msg = format!("Branch {} already exists", branch_name);
            app.add_error(msg.clone());
            return Err(msg.into());
        }
        args.push("-b");
    }
    args.push(branch_name);

    let output = Command::new("git").args(args).output()?;

    if !output.status.success() {
        app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
        return Err("Failed to checkout new branch".into());
    }

    app.add_log("INFO", format!("Checked out new branch: {}", branch_name));
    Ok(String::from_utf8_lossy(output.stdout.as_slice()).to_string())
}

pub fn git_commit_staged_changes(
    app: &mut App,
    commit_title: &str,
    commit_details: &Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut commit_message = commit_title.trim().to_string();
    if let Some(details) = commit_details {
        commit_message.push_str(&format!("\n\n{}", details.trim()));
    }

    let output = Command::new("git")
        .args(["commit", "-m", &commit_message])
        .output()?;
    if !output.status.success() {
        app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
        return Err("Failed to commit changes".into());
    }
    app.add_log("INFO", "Committed changes successfully");

    Ok(())
}

pub fn git_pull_branch(app: &mut App, branch_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(["pull", "origin", branch_name])
        .output()?;

    if !output.status.success() {
        app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
        return Err("Failed to pull branch".into());
    }

    app.add_log("INFO", format!("Pulled branch: {}", branch_name));
    Ok(())
}

pub fn git_has_staged_changes() -> Result<bool, Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .output()?;

    Ok(!output.status.success())
}

pub fn git_stash_pop_autostash_if_exists(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    // Check if AUTOSTASH_NAME exists
    let output = Command::new("git").args(["stash", "list"]).output()?;
    if !output.status.success() {
        app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
        return Err("Failed to list stashes".into());
    }
    if String::from_utf8(output.stdout)?.contains(AUTOSTASH_NAME) {
        app.add_log("INFO", format!("Found stash with name: {}", AUTOSTASH_NAME));
        // Pop AUTOSTASH_NAME: git stash apply stash^{/my_stash_name}
        let output = Command::new("git")
            .args([
                "stash",
                "apply",
                format!("stash^{{/{}}}", AUTOSTASH_NAME).as_str(),
            ])
            .output()?;
        if !output.status.success() {
            app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
            return Err("Failed to pop stash".into());
        }
        app.add_log("INFO", format!("Popped {}", AUTOSTASH_NAME));
    } else {
        app.add_log(
            "INFO",
            format!("No stash found with name: {}", AUTOSTASH_NAME),
        );
    }
    Ok(())
}

pub fn git_stage_and_commit(
    app: &mut App,
    commit_title: &str,
    commit_details: &Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    if git_has_staged_changes()? {
        app.add_log("INFO", "Changes already staged, skipping git add");
    } else {
        let output = Command::new("git").args(["add", "."]).output()?;
        if output.status.success() {
            app.add_log("INFO", "Staged all changes");
        } else {
            app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
            return Err("Failed to stage changes".into());
        }
    }

    git_commit_staged_changes(app, commit_title, commit_details)?;
    app.add_log("INFO", "Committed changes successfully");

    Ok(())
}

pub fn git_push_branch(app: &mut App, branch_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(["push", "origin", branch_name])
        .output()?;
    if !output.status.success() {
        app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
        return Err("Failed to push branch".into());
    }
    app.add_log("INFO", format!("Pushed branch {} to origin", branch_name));
    Ok(())
}

pub fn create_or_update_pull_request(
    app: &mut App,
    title: &str,
    body: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // First check if PR already exists
    let check_output = Command::new("gh")
        .args([
            "pr",
            "list",
            "--state",
            "open",
            "--head",
            &git_current_branch(app)?,
        ])
        .output()?;

    let s = String::from_utf8(check_output.stdout)?.trim().to_string();
    if check_output.status.success()
        && !(s.is_empty() || s.starts_with("no pull requests match your search"))
    {
        // PR exists, update it
        app.add_log("INFO", format!("Existing PR found, updating: {}", s));
        let update_output = Command::new("gh")
            .args([
                "pr",
                "edit",
                "--title",
                title,
                "--body",
                body,
                "--assignee",
                "@me",
            ])
            .output()?;

        if !update_output.status.success() {
            app.add_error(String::from_utf8_lossy(&update_output.stderr).to_string());
            return Err(format!(
                "Failed to update pull request: {}",
                String::from_utf8_lossy(&update_output.stderr)
            )
            .into());
        }
        app.add_log("SUCCESS", "Pull request updated successfully");
    } else {
        // Create new PR
        let create_output = Command::new("gh")
            .args([
                "pr",
                "create",
                "--title",
                title,
                "--body",
                body,
                "--assignee",
                "@me",
            ])
            .output()?;

        if !create_output.status.success() {
            app.add_error(String::from_utf8_lossy(&create_output.stderr).to_string());
            return Err(format!(
                "Failed to create pull request: {}",
                String::from_utf8_lossy(&create_output.stderr)
            )
            .into());
        }
        app.add_log("SUCCESS", "Pull request created successfully");
    }

    // Get and log the PR URL
    let url_output = Command::new("gh")
        .args(["pr", "view", "--json", "url", "--jq", ".url"])
        .output()?;
    if url_output.status.success() {
        if let Ok(url) = String::from_utf8(url_output.stdout) {
            app.add_log("INFO", format!("Pull request URL: {}", url.trim()));
        }
    }
    Ok(())
}
