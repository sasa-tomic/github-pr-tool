use crate::tui::{render_message, App};
use ratatui::style::Color;
use ratatui::{backend::Backend, Terminal};
use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;
use std::process::Command;

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

/// Determines the base branch for comparing changes and generating diffs.
///
/// For an existing PR: Uses the PR's base branch
/// For a new branch: Uses the parent branch this branch was created from
/// Fallback: Uses the repository's main branch
pub fn git_diff_between_branches(
    app: &mut App,
    parent_branch: &str,
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
                        .trim_start_matches("origin/")
                        .to_string();
                    if !parent.is_empty() {
                        app.add_log("INFO", format!("Using parent branch {} for diff", parent));
                        parent
                    } else {
                        app.add_error(format!(
                            "Found invalid empty parent branch, using {} instead",
                            parent_branch
                        ));
                        parent_branch.to_owned()
                    }
                } else {
                    app.add_log(
                        "WARN",
                        format!(
                            "No parent branch marked in git, using branch {}",
                            parent_branch
                        ),
                    );
                    parent_branch.to_owned()
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
                    .trim_start_matches("origin/")
                    .to_string();
                if !parent.is_empty() {
                    app.add_log("INFO", format!("Using parent branch {} for diff", parent));
                    parent
                } else {
                    parent_branch.to_owned()
                }
            } else {
                parent_branch.to_owned()
            }
        }
    } else {
        parent_branch.to_owned()
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

/// Helper function to apply a patch with directory creation support
fn apply_patch_with_directory_creation(
    app: &mut App,
    patch_file: &std::path::PathBuf,
    is_staged: bool,
) -> Result<(), Box<dyn Error>> {
    let patch_path = patch_file.to_str().unwrap();
    let change_type = if is_staged { "staged" } else { "unstaged" };

    // Build git apply command args
    let mut apply_args = vec!["apply"];
    if is_staged {
        apply_args.push("--cached");
    }
    apply_args.extend(["--3way", "--verbose", patch_path]);

    // First try applying with 3-way merge
    let output = Command::new("git").args(&apply_args).output()?;

    if !output.status.success() {
        // If 3-way merge fails, try creating directories first
        app.add_log(
            "INFO",
            format!(
                "3-way merge failed for {} changes, trying to create directories first",
                change_type
            ),
        );

        // Extract directory paths from the patch
        let patch_content = fs_err::read_to_string(patch_file)?;
        let mut dirs_to_create = std::collections::HashSet::new();

        for line in patch_content.lines() {
            if line.starts_with("+++") || line.starts_with("---") {
                if let Some(path) = line.split_whitespace().nth(1) {
                    let path = path.trim_start_matches("b/");
                    if path != "/dev/null" && !path.is_empty() {
                        if let Some(parent) = std::path::Path::new(path).parent() {
                            if !parent.as_os_str().is_empty() {
                                dirs_to_create.insert(parent.to_path_buf());
                            }
                        }
                    }
                }
            }
        }

        // Create directories
        for dir in dirs_to_create {
            if let Err(e) = fs_err::create_dir_all(&dir) {
                app.add_log(
                    "WARN",
                    format!("Failed to create directory {}: {}", dir.display(), e),
                );
            } else {
                app.add_log("INFO", format!("Created directory: {}", dir.display()));
            }
        }

        // Try applying again
        let retry_output = Command::new("git").args(&apply_args).output()?;

        if !retry_output.status.success() {
            app.add_log(
                "WARN",
                format!(
                    "Failed to reapply {} changes after pull - some changes may be lost",
                    change_type
                ),
            );
            app.add_log(
                "INFO",
                format!(
                    "{} patch available at: {}",
                    change_type,
                    patch_file.display()
                ),
            );
            let manual_cmd = if is_staged {
                "git apply --cached --3way <patch-file>"
            } else {
                "git apply --3way <patch-file>"
            };
            app.add_log("INFO", format!("To manually apply: {}", manual_cmd));
            app.add_log(
                "ERROR",
                format!(
                    "Git apply error: {}",
                    String::from_utf8_lossy(&retry_output.stderr)
                ),
            );
        } else {
            app.add_log(
                "INFO",
                format!(
                    "Successfully reapplied {} changes after creating directories",
                    change_type
                ),
            );
        }
    } else {
        app.add_log(
            "INFO",
            format!("Successfully reapplied {} changes", change_type),
        );
    }

    Ok(())
}

pub fn git_fetch_main(
    app: &mut App,
    current_branch: &String,
    main_branch: &String,
) -> Result<(), Box<dyn Error>> {
    if current_branch == main_branch {
        let had_staged_changes = git_has_staged_changes()?;
        if had_staged_changes {
            app.add_log(
                "INFO",
                "Staged changes detected, creating patches to preserve them",
            );

            // Create patch for staged changes
            let staged_patch = Command::new("git")
                .args(["diff", "--staged", "--binary"])
                .output()?;

            // Get unstaged patch only if there are actual unstaged changes
            let unstaged_patch = get_unstaged_patch_if_exists()?;

            // Create directory for patches in .git/
            let git_dir = PathBuf::from(
                String::from_utf8(
                    Command::new("git")
                        .args(["rev-parse", "--git-dir"])
                        .output()?
                        .stdout,
                )?
                .trim(),
            );
            let patches_dir = git_dir.join("gh-autopr-patches");
            fs_err::create_dir_all(&patches_dir)?;

            // Generate timestamp for unique patch filenames
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs();

            let mut staged_patch_file = None;
            let mut unstaged_patch_file = None;

            // Save staged patch to file
            if !staged_patch.stdout.is_empty() {
                let patch_file = patches_dir.join(format!("staged-{}.patch", timestamp));
                fs_err::write(&patch_file, &staged_patch.stdout)?;
                staged_patch_file = Some(patch_file.clone());
                app.add_log(
                    "INFO",
                    format!("Saved staged changes patch to: {}", patch_file.display()),
                );
            }

            // Save unstaged patch to file
            if !unstaged_patch.is_empty() {
                let patch_file = patches_dir.join(format!("unstaged-{}.patch", timestamp));
                fs_err::write(&patch_file, &unstaged_patch)?;
                unstaged_patch_file = Some(patch_file.clone());
                app.add_log(
                    "INFO",
                    format!("Saved unstaged changes patch to: {}", patch_file.display()),
                );
            }

            // Reset all changes to clean working tree for pull
            Command::new("git")
                .args(["reset", "--hard", "HEAD"])
                .output()?;

            // Pull latest changes
            let output = Command::new("git").args(["pull", "origin"]).output()?;
            if !output.status.success() {
                let err = String::from_utf8_lossy(&output.stderr).to_string();
                app.add_error(err.clone());
                return Err(format!("Failed to pull from origin: {}", err).into());
            }
            app.add_log("INFO", "Pulled latest changes from origin");

            // Reapply staged changes
            if let Some(ref patch_file) = staged_patch_file {
                apply_patch_with_directory_creation(app, patch_file, true)?;
            }

            // Reapply unstaged changes
            if let Some(ref patch_file) = unstaged_patch_file {
                apply_patch_with_directory_creation(app, patch_file, false)?;
            }

            // Provide user guidance on patch files
            if staged_patch_file.is_some() || unstaged_patch_file.is_some() {
                app.add_log(
                    "INFO",
                    format!("Patch files stored in: {}", patches_dir.display()),
                );
                app.add_log(
                    "INFO",
                    "You can inspect these patches or apply them manually if needed",
                );
            }

            return Ok(()); // Early return since we already handled the pull
        }
        // Only reach here if no staged changes were detected
        let output = Command::new("git").args(["pull", "origin"]).output()?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr).to_string();
            app.add_error(err.clone());
            return Err(format!("Failed to pull from origin: {}", err).into());
        }
        app.add_log("INFO", "Pulled latest changes from origin");
    } else {
        let output = Command::new("git")
            .args([
                "fetch",
                "origin",
                format!("{}:{}", main_branch, main_branch).as_str(),
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

pub fn git_checkout_branch(app: &mut App, branch_name: &str) -> Result<String, Box<dyn Error>> {
    let output = Command::new("git")
        .args(["checkout", branch_name])
        .output()?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        app.add_error(err.clone());
        return Err(format!("Failed to checkout branch: {}", err).into());
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
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        app.add_error(err.clone());
        return Err(format!("Failed to commit changes: {}", err).into());
    }
    app.add_log("INFO", "Committed changes successfully");

    Ok(())
}

pub fn git_pull_branch(app: &mut App, branch_name: &str) -> Result<(), Box<dyn Error>> {
    let output = Command::new("git")
        .args(["pull", "origin", branch_name])
        .output()?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        app.add_error(err.clone());
        return Err(format!("Failed to pull branch: {}", err).into());
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

/// Get unstaged changes as a binary patch, but only if there are actual unstaged changes.
/// Returns empty Vec if there are no unstaged changes to avoid capturing inverse of staged changes.
pub fn get_unstaged_patch_if_exists() -> Result<Vec<u8>, Box<dyn Error>> {
    let has_unstaged_changes = !Command::new("git")
        .args(["diff", "--quiet"])
        .status()?
        .success();

    if has_unstaged_changes {
        Ok(Command::new("git")
            .args(["diff", "--binary"])
            .output()?
            .stdout)
    } else {
        Ok(Vec::new())
    }
}

pub fn git_stash_pop_autostash_if_exists(app: &mut App) -> Result<(), Box<dyn Error>> {
    // List stashes with format showing only the message
    let output = Command::new("git")
        .args(["stash", "list", "--format=%gD:%gs"]) // %gD gives ref, %gs gives message
        .output()?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        app.add_error(err.clone());
        return Err(format!("Failed to list stashes: {}", err).into());
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
                let err = String::from_utf8_lossy(&output.stderr).to_string();
                app.add_error(err.clone());
                return Err(format!("Failed to apply stash: {}", err).into());
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

/// Updates the original worktree to switch to the PR branch and pull the latest changes.
/// This is called from within a temp worktree before it's cleaned up.
pub fn update_original_worktree_to_pr_branch(
    app: &mut App,
    pr_branch: &str,
    _original_branch: &str,
    original_root: &PathBuf,
) -> Result<(), Box<dyn Error>> {
    let current_dir = std::env::current_dir()?;

    // Switch to original worktree directory
    std::env::set_current_dir(original_root)?;

    let result = (|| -> Result<(), Box<dyn Error>> {
        // First, check if the branch exists locally
        app.add_log(
            "INFO",
            format!("Checking if PR branch '{}' exists locally", pr_branch),
        );
        let check_local_output = Command::new("git")
            .args(["rev-parse", "--verify", pr_branch])
            .output()?;

        if check_local_output.status.success() {
            // Branch exists locally, check for local changes first
            app.add_log(
                "INFO",
                format!("Branch '{}' exists locally, checking out", pr_branch),
            );

            // Check if there are local changes that would be overwritten
            let status_output = Command::new("git")
                .args(["status", "--porcelain"])
                .output()?;

            let has_local_changes = !String::from_utf8_lossy(&status_output.stdout)
                .trim()
                .is_empty();

            if has_local_changes {
                app.add_log("INFO", "Local changes detected, stashing before checkout");
                let stash_output = Command::new("git")
                    .args([
                        "stash",
                        "push",
                        "-m",
                        "gh-autopr: temp stash for branch checkout",
                    ])
                    .output()?;

                if !stash_output.status.success() {
                    app.add_error(format!(
                        "Failed to stash local changes: {}",
                        String::from_utf8_lossy(&stash_output.stderr)
                    ));
                    return Err("Failed to stash local changes".into());
                }
            }

            let checkout_output = Command::new("git").args(["checkout", pr_branch]).output()?;

            if !checkout_output.status.success() {
                app.add_error(format!(
                    "Failed to checkout existing PR branch: {}",
                    String::from_utf8_lossy(&checkout_output.stderr)
                ));
                return Err("Failed to checkout existing PR branch".into());
            }
        } else {
            // Branch doesn't exist locally, try to fetch/create it
            app.add_log(
                "INFO",
                format!(
                    "Branch '{}' doesn't exist locally, fetching from remote",
                    pr_branch
                ),
            );

            // First try to fetch the branch
            let fetch_output = Command::new("git")
                .args(["fetch", "origin", &format!("{}:{}", pr_branch, pr_branch)])
                .output()?;

            if !fetch_output.status.success() {
                // If fetch fails, try to create the branch tracking the remote
                app.add_log("INFO", "Fetch failed, trying to create tracking branch");
                let create_output = Command::new("git")
                    .args([
                        "checkout",
                        "-b",
                        pr_branch,
                        &format!("origin/{}", pr_branch),
                    ])
                    .output()?;

                if !create_output.status.success() {
                    app.add_error(format!(
                        "Failed to create tracking branch: {}",
                        String::from_utf8_lossy(&create_output.stderr)
                    ));
                    return Err("Failed to create tracking branch".into());
                }
            } else {
                // Fetch succeeded, now checkout to the branch
                app.add_log(
                    "INFO",
                    format!("Fetched branch '{}', checking out", pr_branch),
                );
                let checkout_output = Command::new("git").args(["checkout", pr_branch]).output()?;

                if !checkout_output.status.success() {
                    app.add_error(format!(
                        "Failed to checkout fetched PR branch: {}",
                        String::from_utf8_lossy(&checkout_output.stderr)
                    ));
                    return Err("Failed to checkout fetched PR branch".into());
                }
            }
        }

        // Pull the latest changes with rebase for cleaner history
        app.add_log("INFO", "Pulling latest changes from origin (with rebase)");
        let pull_output = Command::new("git")
            .args(["pull", "--rebase", "origin", pr_branch])
            .output()?;

        if !pull_output.status.success() {
            let stderr = String::from_utf8_lossy(&pull_output.stderr);
            if stderr.contains("rebase") || stderr.contains("conflict") {
                app.add_log(
                    "WARN",
                    format!("Rebase failed, falling back to regular pull: {}", stderr),
                );
                // Fallback to regular pull
                let fallback_output = Command::new("git")
                    .args(["pull", "origin", pr_branch])
                    .output()?;

                if !fallback_output.status.success() {
                    app.add_log(
                        "WARN",
                        format!(
                            "Pull failed (this may be normal if branch is up to date): {}",
                            String::from_utf8_lossy(&fallback_output.stderr)
                        ),
                    );
                } else {
                    app.add_log("INFO", "Successfully pulled with regular merge");
                }
            } else {
                app.add_log(
                    "WARN",
                    format!(
                        "Pull failed (this may be normal if branch is up to date): {}",
                        stderr
                    ),
                );
            }
        } else {
            app.add_log("INFO", "Successfully pulled with rebase");
        }

        app.add_log(
            "SUCCESS",
            format!("Original worktree updated to PR branch '{}'", pr_branch),
        );

        // Check if there are any stashes created by our process
        let stash_list_output = Command::new("git")
            .args([
                "stash",
                "list",
                "--grep=gh-autopr: temp stash for branch checkout",
            ])
            .output()?;

        if !String::from_utf8_lossy(&stash_list_output.stdout)
            .trim()
            .is_empty()
        {
            app.add_log(
                "INFO",
                "Local changes were stashed before checkout. Use 'git stash pop' to restore them if needed.",
            );
        }

        Ok(())
    })();

    // Always switch back to the temp worktree directory
    std::env::set_current_dir(&current_dir)?;

    result
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
