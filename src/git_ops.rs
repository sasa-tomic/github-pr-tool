use crate::tui::{render_message, App};
use ratatui::style::Color;
use ratatui::{backend::Backend, Terminal};
use std::collections::HashMap;
use std::error::Error;
use std::io::Write;
use std::process::Command;

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
pub fn truncate_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_owned();
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    } // back up to char boundary
    s[..end].to_owned()
}

/// Get diff between the current branch and its parent/base branch.
pub fn git_diff_between_branches(
    app: &mut App,
    base_branch: &str,
    current_branch: &str,
) -> Result<String, Box<dyn Error>> {
    app.add_log(
        "INFO",
        format!(
            "Comparing {} against base branch: {}",
            current_branch, base_branch
        ),
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
            let err = String::from_utf8_lossy(&output.stderr).to_string();
            app.add_error(err.clone());
            return Err(format!("Failed to set origin HEAD: {}", err).into());
        }

        main_branch_output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "origin/HEAD"])
            .output()?;

        if !main_branch_output.status.success() {
            let err = String::from_utf8_lossy(&main_branch_output.stderr).to_string();
            app.add_error(err.clone());
            return Err(format!("Failed to determine main branch: {}", err).into());
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
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        app.add_error(err.clone());
        return Err(format!("Failed to get current branch: {}", err).into());
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
    main_branch: &str, // usually "main" or "master"
    child: &str,
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

    // 2. fall back to "nearest ancestor" among local branches
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
        let up = String::from_utf8_lossy(&out.stdout)
            .trim()
            .trim_start_matches("origin/")
            .to_owned();
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

/// Fetch/pull latest changes from origin.
/// When on main branch: just fetch (don't pull - we'll work in temp worktree).
/// When on feature branch: fetch the main branch for comparison.
pub fn git_fetch_main(
    app: &mut App,
    current_branch: &str,
    main_branch: &str,
) -> Result<(), Box<dyn Error>> {
    if current_branch == main_branch {
        // Just fetch, don't pull - temp worktree handles dirty state
        let output = Command::new("git").args(["fetch", "origin"]).output()?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr).to_string();
            app.add_error(err.clone());
            return Err(format!("Failed to fetch from origin: {}", err).into());
        }
        app.add_log("INFO", "Fetched latest changes from origin");
    } else {
        let output = Command::new("git")
            .args([
                "fetch",
                "origin",
                &format!("{}:{}", main_branch, main_branch),
            ])
            .output()?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr).to_string();
            app.add_error(err.clone());
            return Err(format!("Failed to fetch main branch: {}", err).into());
        }
        app.add_log("INFO", format!("Fetched latest {} branch", main_branch));
    }

    Ok(())
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
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        app.add_error(err.clone());
        return Err(format!("Failed to commit changes: {}", err).into());
    }
    app.add_log("INFO", "Committed changes successfully");

    Ok(())
}

pub fn git_has_staged_changes() -> Result<bool, Box<dyn Error>> {
    let output = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .output()?;

    Ok(!output.status.success())
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
            let err = String::from_utf8_lossy(&output.stderr).to_string();
            app.add_error(err.clone());
            return Err(format!("Failed to stage changes: {}", err).into());
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
        // If no upstream exists, set it up with the --set-upstream flag
        push_args.extend(["--set-upstream", "origin", branch_name]);
        app.add_log("INFO", "Setting up upstream tracking branch");
    } else {
        push_args.extend(["origin", branch_name]);
    }

    let output = Command::new("git").args(&push_args).output()?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        app.add_error(err.clone());
        return Err(format!("Failed to push branch: {}", err).into());
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

use std::path::PathBuf;

/// Updates the original worktree to the PR branch after temp worktree cleanup.
/// Called after the temp worktree is dropped, so we're already back in the original worktree.
///
/// The behavior depends on what was committed to the PR:
/// - If there were STAGED changes: only those went to PR, so discard staged but KEEP unstaged
/// - If there were NO staged changes: all unstaged changes went to PR, so discard everything
pub fn update_original_worktree_to_pr_branch(
    app: &mut App,
    pr_branch: &str,
    original_root: &PathBuf,
    had_staged_changes: bool,
) -> Result<(), Box<dyn Error>> {
    std::env::set_current_dir(original_root)?;

    if had_staged_changes {
        // Only staged changes went to PR. Keep unstaged changes.
        // 1. Save unstaged changes (working tree vs index), including binary files
        let unstaged_diff = Command::new("git").args(["diff", "--binary"]).output()?;
        if !unstaged_diff.status.success() {
            app.add_log(
                "WARN",
                format!(
                    "Failed to capture unstaged changes: {}",
                    String::from_utf8_lossy(&unstaged_diff.stderr)
                ),
            );
        }
        let unstaged_patch = unstaged_diff.stdout;

        // 2. Hard reset to discard staged changes
        let reset_output = Command::new("git")
            .args(["reset", "--hard", "HEAD"])
            .output()?;
        if !reset_output.status.success() {
            app.add_log(
                "WARN",
                format!(
                    "Failed to reset: {}",
                    String::from_utf8_lossy(&reset_output.stderr)
                ),
            );
        }

        // 3. Checkout PR branch
        checkout_pr_branch(app, pr_branch)?;

        // 4. Re-apply unstaged changes
        if !unstaged_patch.is_empty() {
            let mut child = Command::new("git")
                .args(["apply", "--3way", "-"])
                .stdin(std::process::Stdio::piped())
                .spawn()?;
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(&unstaged_patch);
            }
            let status = child.wait()?;
            if status.success() {
                app.add_log("INFO", "Restored unstaged changes");
            } else {
                app.add_log(
                    "WARN",
                    "Some unstaged changes could not be restored cleanly",
                );
            }
        }
    } else {
        // All changes (unstaged + untracked) went to PR. Discard everything.
        let reset_output = Command::new("git")
            .args(["reset", "--hard", "HEAD"])
            .output()?;
        if !reset_output.status.success() {
            app.add_log(
                "WARN",
                format!(
                    "Failed to reset: {}",
                    String::from_utf8_lossy(&reset_output.stderr)
                ),
            );
        }

        // Remove untracked files (they were committed to PR)
        let clean_output = Command::new("git").args(["clean", "-fd"]).output()?;
        if !clean_output.status.success() {
            app.add_log(
                "WARN",
                format!(
                    "Failed to clean untracked files: {}",
                    String::from_utf8_lossy(&clean_output.stderr)
                ),
            );
        }

        // Checkout PR branch
        checkout_pr_branch(app, pr_branch)?;
    }

    app.add_log("SUCCESS", format!("Switched to branch '{}'", pr_branch));
    Ok(())
}

/// Helper to checkout the PR branch (fetch from remote if needed)
fn checkout_pr_branch(app: &mut App, pr_branch: &str) -> Result<(), Box<dyn Error>> {
    // Check if branch exists locally
    let branch_exists = Command::new("git")
        .args(["rev-parse", "--verify", pr_branch])
        .status()?
        .success();

    if branch_exists {
        let output = Command::new("git").args(["checkout", pr_branch]).output()?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            app.add_error(format!("Failed to checkout {}: {}", pr_branch, err));
            return Err(format!("Failed to checkout branch: {}", err).into());
        }
    } else {
        // Fetch and checkout from remote
        let _ = Command::new("git")
            .args(["fetch", "origin", &format!("{}:{}", pr_branch, pr_branch)])
            .output();

        let output = Command::new("git").args(["checkout", pr_branch]).output()?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            app.add_error(format!("Failed to checkout {}: {}", pr_branch, err));
            return Err(format!("Failed to checkout branch: {}", err).into());
        }
    }

    Ok(())
}

/// Get merged PRs and their associated branches
pub fn get_merged_prs_and_branches(
    app: &mut App,
) -> Result<HashMap<String, String>, Box<dyn Error>> {
    app.add_log("INFO", "Fetching merged PRs...");

    let output = Command::new("gh")
        .args([
            "pr",
            "list",
            "--state",
            "merged",
            "--json",
            "headRefName,number,title",
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        app.add_error(format!("Failed to get merged PRs: {}", stderr));
        return Err("Failed to get merged PRs".into());
    }

    let json_str = String::from_utf8(output.stdout)?;
    let prs: Vec<serde_json::Value> = serde_json::from_str(&json_str)?;

    let mut merged_branches = HashMap::new();
    for pr in prs {
        if let (Some(branch), Some(number)) = (pr["headRefName"].as_str(), pr["number"].as_u64()) {
            merged_branches.insert(branch.to_string(), format!("PR #{}", number));
        }
    }

    app.add_log(
        "INFO",
        format!("Found {} merged PRs", merged_branches.len()),
    );
    Ok(merged_branches)
}

/// Get local branches and their remote tracking branches
pub fn get_local_branches_with_remotes(
    app: &mut App,
) -> Result<HashMap<String, Option<String>>, Box<dyn Error>> {
    app.add_log("INFO", "Getting local branches...");

    let output = Command::new("git").args(["branch", "-vv"]).output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        app.add_error(format!("Failed to get local branches: {}", stderr));
        return Err("Failed to get local branches".into());
    }

    let branch_output = String::from_utf8(output.stdout)?;
    let mut branches = HashMap::new();

    for line in branch_output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Parse git branch -vv output format
        // Example: "* main                 1234567 [origin/main] Latest commit"
        // Example: "  feature-branch       abcdef1 [origin/feature-branch: ahead 1] Add feature"

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }

        let branch_name = if parts[0] == "*" {
            parts[1] // Current branch, name is in second position
        } else {
            parts[0] // Non-current branch, name is in first position
        };

        // Skip if this is the current branch indicator
        if branch_name == "*" {
            continue;
        }

        // Look for remote tracking branch in brackets
        let remote_branch = if let Some(bracket_start) = line.find('[') {
            if let Some(bracket_end) = line.find(']') {
                let remote_info = &line[bracket_start + 1..bracket_end];
                // Extract just the remote branch name (before any : or other info)
                let remote_branch = remote_info.split(':').next().unwrap_or(remote_info).trim();
                Some(remote_branch.to_string())
            } else {
                None
            }
        } else {
            None
        };

        branches.insert(branch_name.to_string(), remote_branch);
    }

    app.add_log("INFO", format!("Found {} local branches", branches.len()));
    Ok(branches)
}

/// Check if a remote branch exists
pub fn remote_branch_exists(_app: &mut App, remote_branch: &str) -> Result<bool, Box<dyn Error>> {
    let output = Command::new("git")
        .args(["ls-remote", "--exit-code", "origin", remote_branch])
        .output()?;

    Ok(output.status.success())
}

/// Delete a local branch
pub fn delete_local_branch(app: &mut App, branch_name: &str) -> Result<(), Box<dyn Error>> {
    app.add_log("INFO", format!("Deleting local branch: {}", branch_name));

    let output = Command::new("git")
        .args(["branch", "-D", branch_name])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        app.add_error(format!(
            "Failed to delete branch {}: {}",
            branch_name, stderr
        ));
        return Err(format!("Failed to delete branch {}", branch_name).into());
    }

    app.add_log("SUCCESS", format!("Deleted local branch: {}", branch_name));
    Ok(())
}

/// Main function to prune merged branches
pub fn prune_merged_branches(app: &mut App) -> Result<(), Box<dyn Error>> {
    app.add_log("INFO", "Starting branch pruning process...");

    // Get merged PRs and their branches
    let merged_prs = get_merged_prs_and_branches(app)?;

    // Get local branches with their remotes
    let local_branches = get_local_branches_with_remotes(app)?;

    // Get current branch to avoid deleting it
    let current_branch = git_current_branch(app)?;

    // Get main branch to avoid deleting it
    let main_branch = git_main_branch(app).unwrap_or_else(|_| "main".to_string());

    let mut deleted_count = 0;
    let mut skipped_count = 0;

    for (local_branch, remote_branch_opt) in local_branches {
        // Skip current branch
        if local_branch == current_branch {
            app.add_log("INFO", format!("Skipping current branch: {}", local_branch));
            skipped_count += 1;
            continue;
        }

        // Skip main branch
        if local_branch == main_branch {
            app.add_log("INFO", format!("Skipping main branch: {}", local_branch));
            skipped_count += 1;
            continue;
        }

        // Check if this branch corresponds to a merged PR
        if merged_prs.contains_key(&local_branch) {
            // Branch was merged via PR, safe to delete
            match delete_local_branch(app, &local_branch) {
                Ok(_) => {
                    let pr_info = merged_prs.get(&local_branch).unwrap();
                    app.add_log("SUCCESS", format!("Deleted {} ({})", local_branch, pr_info));
                    deleted_count += 1;
                }
                Err(e) => {
                    app.add_error(format!("Failed to delete {}: {}", local_branch, e));
                }
            }
        } else if let Some(remote_branch) = remote_branch_opt {
            // Check if remote branch still exists
            match remote_branch_exists(app, &remote_branch) {
                Ok(false) => {
                    // Remote branch doesn't exist, likely merged and deleted
                    app.add_log(
                        "INFO",
                        format!(
                            "Remote branch {} no longer exists, deleting local branch {}",
                            remote_branch, local_branch
                        ),
                    );
                    match delete_local_branch(app, &local_branch) {
                        Ok(_) => {
                            deleted_count += 1;
                        }
                        Err(e) => {
                            app.add_error(format!("Failed to delete {}: {}", local_branch, e));
                        }
                    }
                }
                Ok(true) => {
                    app.add_log(
                        "INFO",
                        format!(
                            "Remote branch {} still exists, keeping local branch {}",
                            remote_branch, local_branch
                        ),
                    );
                    skipped_count += 1;
                }
                Err(e) => {
                    app.add_error(format!(
                        "Failed to check remote branch {}: {}",
                        remote_branch, e
                    ));
                    skipped_count += 1;
                }
            }
        } else {
            // No remote tracking branch, skip
            app.add_log(
                "INFO",
                format!("No remote tracking branch for {}, skipping", local_branch),
            );
            skipped_count += 1;
        }
    }

    app.add_log(
        "SUCCESS",
        format!(
            "Branch pruning completed: {} deleted, {} skipped",
            deleted_count, skipped_count
        ),
    );
    Ok(())
}

#[cfg(test)]
#[path = "git_ops/tests.rs"]
mod tests;
