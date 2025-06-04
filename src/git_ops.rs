use crate::tui::{render_message, App};
use once_cell::sync::OnceCell;
use ratatui::style::Color;
use ratatui::{backend::Backend, Terminal};
use std::error::Error;
use std::{process::Command, sync::Mutex};

const AUTOCOMMIT_BRANCH_NAME: &str = "gh-autopr-index-autocommit";
const AUTOSTASH_NAME: &str = "gh-autopr-index-autostash";
const MAX_DIFF_BYTES: usize = 200 * 1024; // 200 KiB

pub fn git_ensure_in_repo(app: &mut App) -> Result<(), Box<dyn Error>> {
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
) -> Result<(), Box<dyn Error>> {
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

pub fn git_cd_to_repo_root(app: &mut App) -> Result<(), Box<dyn Error>> {
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

/// Return either
/// 1. the diff of staged/index changes against `merge_base` (or `HEAD`), or
/// 2. if nothing is staged, the diff of **working-tree** changes against `merge_base`.
///
/// The result is truncated to `MAX_DIFF_BYTES` **on a character boundary**
/// to keep it AI-friendly.
pub fn git_diff_uncommitted(app: &mut App, current_branch: &str) -> Result<String, Box<dyn Error>> {
    let pathspec = ["--", ".", ":!*.lock"]; // exclude *.lock anywhere

    // 1. staged changes first
    if let Some(diff) = git_run_diff(app, true, current_branch, &pathspec)? {
        return Ok(truncate_utf8(&diff, MAX_DIFF_BYTES));
    }

    // 2. otherwise fall back to working-tree changes
    let diff = git_run_diff(app, false, current_branch, &pathspec)?.unwrap_or_default(); // may be empty
    Ok(truncate_utf8(&diff, MAX_DIFF_BYTES))
}

/// Helper: run `git diff`, returning `Ok(Some(diff))` if diff is non-empty.
fn git_run_diff(
    app: &mut App,
    staged: bool,
    base: &str,
    pathspec: &[&str],
) -> Result<Option<String>, Box<dyn Error>> {
    let mut args = vec!["diff"];
    if staged {
        args.push("--staged"); // alias for `--cached`
    }
    args.push(base);
    args.extend_from_slice(pathspec);

    let out = Command::new("git").args(&args).output()?;
    if !out.status.success() {
        app.add_error(String::from_utf8_lossy(&out.stderr).to_string());
        return Err("git diff failed".into());
    }

    let diff = String::from_utf8(out.stdout)?.trim().to_owned();
    Ok(if diff.is_empty() { None } else { Some(diff) })
}

/// Truncate **without** splitting UTF-8 characters.
fn truncate_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_owned();
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    } // back up to char boundary
    s[..end].to_owned()
}

/// Determines the base branch for comparing changes and generating diffs.
///
/// For an existing PR: Uses the PR's base branch
/// For a new branch: Uses the parent branch this branch was created from
/// Fallback: Uses the repository's main branch
pub fn git_diff_between_branches(
    app: &mut App,
    main_branch: &str,
    current_branch: &String,
) -> Result<String, Box<dyn Error>> {
    // First try to get base branch from existing PR
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
                app.add_log(
                    "INFO",
                    format!("Using existing PR base branch {} for diff", base),
                );
                base
            } else {
                // If no PR exists, try to find the parent branch this was branched from
                let parent_branch_output = Command::new("git")
                    .args([
                        "rev-parse",
                        "--abbrev-ref",
                        &format!("{}@{{-1}}", current_branch),
                    ])
                    .output()?;

                if parent_branch_output.status.success() {
                    let parent = String::from_utf8(parent_branch_output.stdout)?
                        .trim()
                        .to_string();
                    if !parent.is_empty() {
                        app.add_log("INFO", format!("Using parent branch {} for diff", parent));
                        parent
                    } else {
                        app.add_error(format!(
                            "Found invalid empty parent branch, using {} instead",
                            main_branch
                        ));
                        main_branch.to_owned()
                    }
                } else {
                    app.add_error(format!(
                        "No parent branch found, using {} branch",
                        main_branch
                    ));
                    main_branch.to_owned()
                }
            }
        } else {
            // No PR exists, try to find parent branch
            let parent_branch_output = Command::new("git")
                .args([
                    "rev-parse",
                    "--abbrev-ref",
                    &format!("{}@{{-1}}", current_branch),
                ])
                .output()?;

            if parent_branch_output.status.success() {
                let parent = String::from_utf8(parent_branch_output.stdout)?
                    .trim()
                    .to_string();
                if !parent.is_empty() {
                    app.add_log("INFO", format!("Using parent branch {} for diff", parent));
                    parent
                } else {
                    main_branch.to_owned()
                }
            } else {
                main_branch.to_owned()
            }
        }
    } else {
        main_branch.to_owned()
    };

    app.add_log(
        "INFO",
        format!("Comparing against base branch: {}", base_branch),
    );

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

pub fn git_main_branch(app: &mut App) -> Result<String, Box<dyn Error>> {
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

pub fn git_current_branch(app: &mut App) -> Result<String, Box<dyn Error>> {
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

/// Guess the branch that `child` was forked from.
///
/// Behaviour
/// ----------
/// * If `child == primary`           → `Ok(primary)`
/// * If `child` has an upstream set  → that upstream (unless it *is* `primary`)
/// * Otherwise                       → the **nearest** local branch that is an
///   ancestor of `child` (smallest commit distance).
///
/// This heuristic matches stacked-PR workflows where each new branch is created
/// with `git switch -c <new> --track <parent>` or `git checkout -b <new> <parent>`.
pub fn discover_parent_branch(
    app: &mut App,
    child: &str,
    main_branch: &str, // usually "main" or "master"
) -> Result<String, Box<dyn Error>> {
    if child == main_branch {
        return Ok(main_branch.to_owned());
    }

    // 1. explicit upstream, if configured
    if let Some(up) = upstream_of(child)? {
        if up != main_branch {
            app.add_log("INFO", format!("Found upstream branch: {}", up));
            return Ok(up);
        }
    }

    // 2. fall back to “nearest ancestor” among local branches
    let local_branches = for_each_local_ref()?;
    let mut best: Option<(String, usize)> = None; // (branch, distance)

    for cand in &local_branches {
        if cand == child {
            continue;
        }

        // Skip candidates that are not ancestors of `child`.
        let is_ancestor = Command::new("git")
            .args(["merge-base", "--is-ancestor", cand, child])
            .status()?
            .success();

        if !is_ancestor {
            continue;
        }

        // Distance = #commits child is ahead of cand.
        let dist = commit_distance(cand, child)?;
        match best {
            Some((_, d)) if d <= dist => {} // keep closer branch
            _ => best = Some((cand.clone(), dist)),
        }
    }

    let result = if let Some((branch, _)) = best {
        app.add_log("INFO", format!("Found parent branch: {}", branch));
        branch
    } else {
        app.add_log("INFO", "No parent branch found, using main branch");
        main_branch.to_owned()
    };
    Ok(result)
}

/* ─────────────────────────── helpers ─────────────────────────────────────── */

// Upstream branch, if any (e.g. "origin/main" or "branch_a")
fn upstream_of(branch: &str) -> Result<Option<String>, Box<dyn Error>> {
    let spec = format!("{branch}@{{upstream}}");
    let out = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", &spec])
        .output()?;

    if out.status.success() {
        let up = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        Ok(if up.is_empty() { None } else { Some(up) })
    } else {
        Ok(None)
    }
}

// List local branch names (short form, no remotes)
fn for_each_local_ref() -> Result<Vec<String>, Box<dyn Error>> {
    let out = Command::new("git")
        .args(["for-each-ref", "--format=%(refname:short)", "refs/heads/"])
        .output()?;

    if !out.status.success() {
        return Err("git for-each-ref failed".into());
    }
    Ok(String::from_utf8(out.stdout)?
        .lines()
        .map(|s| s.to_owned())
        .collect())
}

// Count commits reachable from `to` and not from `from`
fn commit_distance(from: &str, to: &str) -> Result<usize, Box<dyn Error>> {
    let range = format!("{from}..{to}");
    let out = Command::new("git")
        .args(["rev-list", "--count", &range])
        .output()?;

    if !out.status.success() {
        return Err("git rev-list failed".into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().parse()?)
}

pub fn git_fetch_main(
    app: &mut App,
    current_branch: &String,
    main_branch: &String,
) -> Result<(), Box<dyn Error>> {
    if current_branch == main_branch {
        let had_staged_changes = git_has_staged_changes()?;
        if had_staged_changes {
            app.add_log("INFO", "Staged changes detected, stashing in temp branch");
            git_checkout_new_branch(app, AUTOCOMMIT_BRANCH_NAME, current_branch, true)?;
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

pub fn git_checkout_branch(app: &mut App, branch_name: &str) -> Result<String, Box<dyn Error>> {
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
    current_branch: &str,
    force_reset: bool,
) -> Result<String, Box<dyn Error>> {
    // Check if branch exists (unless force_reset is true)
    if !force_reset {
        let exists = Command::new("git")
            .args(["rev-parse", "--verify", branch_name])
            .status()?
            .success();

        if exists {
            let e = format!(
                "branch \"{branch_name}\" already exists (pass force_reset=true to rewrite)"
            );
            app.add_error(e.clone());
            return Err(e.into());
        }
    }

    // Create or reset branch to current_branch's tip
    let output = Command::new("git")
        .args(["checkout", "-B", branch_name, current_branch])
        .output()?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        app.add_error(err.clone());
        return Err(err.into());
    }

    app.add_log(
        "INFO",
        format!("Created branch \"{branch_name}\" from \"{current_branch}\""),
    );

    Ok(branch_name.to_owned())
}
pub fn git_commit_staged_changes(
    app: &mut App,
    commit_title: &str,
    commit_details: &Option<String>,
) -> Result<(), Box<dyn Error>> {
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

pub fn git_pull_branch(app: &mut App, branch_name: &str) -> Result<(), Box<dyn Error>> {
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

pub fn git_has_staged_changes() -> Result<bool, Box<dyn Error>> {
    let output = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .output()?;

    Ok(!output.status.success())
}

pub fn git_stash_pop_autostash_if_exists(app: &mut App) -> Result<(), Box<dyn Error>> {
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
        if parts.len() >= 2
            && parts[1] == format!("On {}: {}", AUTOCOMMIT_BRANCH_NAME, AUTOSTASH_NAME)
        {
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
) -> Result<(), Box<dyn Error>> {
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

pub fn git_push_branch(app: &mut App, branch_name: &str) -> Result<(), Box<dyn Error>> {
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

/// Creates or updates a pull request.
///
/// # Arguments
/// * `app` - Application state
/// * `title` - PR title
/// * `body` - PR description
/// * `update_pr` - Whether to update existing PR instead of creating new one
/// * `ready` - Whether to create as ready for review (false = draft)
/// * `base_branch` - The target (base) branch for the PR
/// * `current_branch` - The source (head) branch for the PR
pub fn create_or_update_pull_request(
    app: &mut App,
    title: &str,
    body: &str,
    update_pr: bool,
    ready: bool,
    base_branch: &str,
    current_branch: &str,
) -> Result<(), Box<dyn Error>> {
    app.add_log(
        "INFO",
        format!(
            "{} PR from {} into {}",
            if update_pr {
                "Updating/checking"
            } else {
                "Creating"
            },
            current_branch,
            base_branch
        ),
    );

    // Check for existing PR
    let check_output = Command::new("gh")
        .args(["pr", "list", "--state", "open", "--head", current_branch])
        .output()?;

    let s = String::from_utf8(check_output.stdout)?.trim().to_string();
    let pr_exists = check_output.status.success()
        && !(s.is_empty() || s.starts_with("no pull requests match your search"));

    let should_update = update_pr && pr_exists;

    if should_update {
        let args = vec![
            "pr",
            "edit",
            "--title",
            title,
            "--body",
            body,
            "--add-assignee",
            "@me",
        ];

        let update_output = Command::new("gh").args(&args).output()?;

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
        let mut args = vec![
            "pr",
            "create",
            "--title",
            title,
            "--body",
            body,
            "--assignee",
            "@me",
            "--head",
            current_branch,
            "--base",
            base_branch,
        ];

        if !ready {
            args.push("--draft");
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
pub fn git_list_issues(app: &mut App) -> Result<String, Box<dyn Error>> {
    // Initialize cache if not already initialized
    let cache = ISSUES_CACHE.get_or_init(|| Mutex::new(None));
    let mut cache = cache.lock().unwrap();

    // Return cached data if available
    if let Some(cached_data) = cache.as_ref() {
        app.add_log("INFO", "Using cached GitHub issues");
        return Ok(cached_data.clone());
    }

    // Cache miss - fetch from GitHub
    let output = Command::new("gh")
        .args(["issue", "list", "--json", "number,title,labels,body"])
        .output()?;

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        // Handle specific error cases
        if stderr.contains("has disabled issues") {
            app.add_log("WARN", "Repository has issues disabled");
            let empty_list = "[]".to_string();
            *cache = Some(empty_list.clone());
            return Ok(empty_list);
        }
        app.add_error(stderr);
        return Err("Failed to list issues".into());
    }

    let json_str = String::from_utf8(output.stdout)?;
    app.add_log("INFO", "Successfully retrieved fresh GitHub issues");

    // Update cache
    *cache = Some(json_str.clone());

    Ok(json_str)
}

static ISSUES_CACHE: OnceCell<Mutex<Option<String>>> = OnceCell::new();
