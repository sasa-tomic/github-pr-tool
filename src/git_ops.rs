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
    const MAX_DIFF_SIZE: usize = 200 * 1024; // 200KB limit, many AIs don't handle more than this
    if diff_context.len() > MAX_DIFF_SIZE {
        return Ok(diff_context[..MAX_DIFF_SIZE].to_string());
    }
    Ok(diff_context)
}

pub fn git_diff_between_branches(
    app: &mut App,
    main_branch: &str,
    current_branch: &String,
) -> Result<String, Box<dyn std::error::Error>> {
    // Check if this branch has an existing PR to determine base branch
    let base_branch_output = Command::new("gh")
        .args([
            "pr",
            "list",
            "--state",
            "open",
            "--head",
            current_branch,
            "--json",
            "baseRefName",
        ])
        .output()?;

    let base_branch = if base_branch_output.status.success() {
        let json_str = String::from_utf8(base_branch_output.stdout)?;
        if !json_str.trim().is_empty() && json_str != "[]" {
            if let Some(base) = json_str
                .lines()
                .next()
                .and_then(|line| serde_json::from_str::<Vec<serde_json::Value>>(line).ok())
                .and_then(|prs| prs.first().cloned())
                .and_then(|pr| pr["baseRefName"].as_str().map(|s| s.to_string()))
            {
                app.add_log("INFO", format!("Using PR base branch {} for diff", base));
                base
            } else {
                main_branch.to_owned()
            }
        } else {
            main_branch.to_owned()
        }
    } else {
        main_branch.to_owned()
    };

    let output = Command::new("git")
        .args([
            "diff",
            &format!("{}...{}", base_branch, current_branch),
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
    // List stashes with format showing only the message
    let output = Command::new("git")
        .args(["stash", "list", "--format=%gD:%gs"]) // %gD gives ref, %gs gives message
        .output()?;

    if !output.status.success() {
        app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
        return Err("Failed to list stashes".into());
    }

    let stash_list = String::from_utf8(output.stdout)?;
    for line in stash_list.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 2 && parts[1] == AUTOSTASH_NAME {
            app.add_log("INFO", format!("Found stash with name: {}", AUTOSTASH_NAME));
            // Use the exact stash reference (parts[0] contains stash@{N})
            let output = Command::new("git")
                .args(["stash", "apply", parts[0]])
                .output()?;

            if !output.status.success() {
                app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
                return Err("Failed to apply stash".into());
            }
            app.add_log("INFO", format!("Applied {}", AUTOSTASH_NAME));
            return Ok(());
        }
    }

    app.add_log(
        "INFO",
        format!("No stash found with name: {}", AUTOSTASH_NAME),
    );
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
    // Check if branch already has upstream tracking
    let check_upstream = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", &format!("{branch_name}@{{u}}")])
        .output()?;

    let has_upstream = check_upstream.status.success();
    let mut push_args = vec!["push"];

    if !has_upstream {
        // If no upstream exists, set it up with -u flag
        push_args.extend(["--set-upstream", "origin", branch_name]);
        app.add_log("INFO", "Setting up upstream tracking branch");
    } else {
        push_args.extend(["origin", branch_name]);
    }

    let output = Command::new("git").args(&push_args).output()?;

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
    update_pr: bool,
    ready: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let current_branch = git_current_branch(app)?;

    // Check if PR exists and get base branch
    let check_output = Command::new("gh")
        .args([
            "pr",
            "list",
            "--state",
            "open",
            "--head",
            &current_branch,
            "--json",
            "baseRefName",
        ])
        .output()?;

    let s = String::from_utf8(check_output.stdout)?.trim().to_string();
    let pr_exists = check_output.status.success()
        && !(s.is_empty() || s.starts_with("no pull requests match your search"));

    let base_branch = if check_output.status.success() && !s.is_empty() && s != "[]" {
        serde_json::from_str::<Vec<serde_json::Value>>(&s)
            .ok()
            .and_then(|prs| prs.first().cloned())
            .and_then(|pr| pr["baseRefName"].as_str().map(|s| s.to_string()))
    } else {
        None
    };

    let should_update = update_pr && pr_exists;

    if should_update {
        let update_output = Command::new("gh")
            .args([
                "pr",
                "edit",
                "--title",
                title,
                "--body",
                body,
                "--add-assignee",
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
    } else if update_pr {
        app.add_error("No existing PR found to update".to_string());
        return Err("No existing PR found to update".into());
    } else {
        // Create new PR
        let mut args = vec![
            "pr".to_string(),
            "create".to_string(),
            "--title".to_string(),
            title.to_string(),
            "--body".to_string(),
            body.to_string(),
            "--assignee".to_string(),
            "@me".to_string(),
        ];

        // Add base branch if found from existing PR
        if let Some(base) = base_branch {
            args.push("--base".to_string());
            args.push(base.clone());
            app.add_log("INFO", format!("Using {} as base branch", base));
        }
        if !ready {
            args.push("--draft".to_string());
        }

        let create_output = Command::new("gh").args(&args).output()?;

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

// Example GitHub issues JSON output:
/*
[
  {
    "body": "This is a body of the GH issue.",
    "labels": [
      {
        "id": "LA_kwDOOTdaS88AAAAB9JPIwX",
        "name": "bug",
        "description": "Something isn't working",
        "color": "d73a4a"
      }
    ],
    "number": 42,
    "title": "This is a title of the GH issue."
  }
]
*/
pub fn git_list_issues(app: &mut App) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("gh")
        .args(["issue", "list", "--json", "number,title,labels,body"])
        .output()?;

    if !output.status.success() {
        app.add_error(String::from_utf8_lossy(&output.stderr).to_string());
        return Err("Failed to list issues".into());
    }

    let json_str = String::from_utf8(output.stdout)?;
    app.add_log("INFO", "Successfully retrieved GitHub issues");
    Ok(json_str)
}
