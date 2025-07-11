use crate::git_ops::get_unstaged_patch_if_exists;
use crate::App;
use std::error::Error;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Try to reuse recent patches created by git_fetch_main() to avoid duplication
/// Returns (staged_patch, unstaged_patch) if recent patches found, otherwise empty vecs
fn try_reuse_recent_patches(patches_dir: &Path) -> Result<(Vec<u8>, Vec<u8>), Box<dyn Error>> {
    if !patches_dir.exists() {
        return Ok((Vec::new(), Vec::new()));
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    const PATCH_REUSE_THRESHOLD_SECS: u64 = 10; // Only reuse patches created within last 10 seconds

    let mut staged_patch = Vec::new();
    let mut unstaged_patch = Vec::new();

    // Look for recent patch files
    if let Ok(entries) = fs_err::read_dir(patches_dir) {
        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() && path.extension().is_some_and(|ext| ext == "patch") {
                if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                    // Extract timestamp from filename (format: staged-TIMESTAMP.patch or unstaged-TIMESTAMP.patch)
                    if let Some(timestamp_str) =
                        filename.split('-').nth(1).and_then(|s| s.split('.').next())
                    {
                        if let Ok(timestamp) = timestamp_str.parse::<u64>() {
                            if now - timestamp <= PATCH_REUSE_THRESHOLD_SECS {
                                // Recent patch found, read it
                                let patch_content = fs_err::read(&path)?;

                                if filename.starts_with("staged-") {
                                    staged_patch = patch_content;
                                } else if filename.starts_with("unstaged-") {
                                    unstaged_patch = patch_content;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok((staged_patch, unstaged_patch))
}

/// RAII guard for the temp worktree
pub struct TempWorktree {
    path: PathBuf,
    orig_root: PathBuf,
    orig_branch: String,
}

impl TempWorktree {
    /// Enter a detached worktree that lives in `.git/autopr-wt-<uuid>`.
    pub fn enter() -> Result<Self, Box<dyn Error>> {
        // 1. capture original location + branch + changes ------------------------
        let orig_root = PathBuf::from(
            String::from_utf8(
                Command::new("git")
                    .args(["rev-parse", "--show-toplevel"])
                    .output()?
                    .stdout,
            )?
            .trim(),
        );
        let orig_branch = String::from_utf8(
            Command::new("git")
                .args(["rev-parse", "--abbrev-ref", "HEAD"])
                .output()?
                .stdout,
        )?
        .trim()
        .to_owned();

        // Try to reuse recent patches created by git_fetch_main() first
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
        let (staged_patch, unstaged_patch) = try_reuse_recent_patches(&patches_dir)?;

        // If no recent patches found, create them fresh
        let (staged_patch, unstaged_patch) = if staged_patch.is_empty() && unstaged_patch.is_empty()
        {
            let staged = Command::new("git")
                .args(["diff", "--staged", "--binary"])
                .output()?
                .stdout;
            let unstaged = get_unstaged_patch_if_exists()?;
            (staged, unstaged)
        } else {
            (staged_patch, unstaged_patch)
        };

        let untracked_list = String::from_utf8(
            Command::new("git")
                .args(["ls-files", "--others", "--exclude-standard", "-z"])
                .output()?
                .stdout,
        )?;

        // 2. construct a path on the *same* filesystem (inside .git) -------------
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis();
        let path = git_dir.join(format!("autopr-wt-{}", ts));

        // 3. create detached worktree at HEAD ------------------------------------
        let out = Command::new("git")
            .args(["worktree", "add", "--detach", path.to_str().unwrap()])
            .output()?;
        if !out.status.success() {
            return Err(format!(
                "git worktree add failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )
            .into());
        }

        // 4. hop into that directory --------------------------------------------
        std::env::set_current_dir(&path)?;

        // 5. make the worktree point at the **same branch name** the user had.
        //    Use --force so this works even if that branch is already active
        //    in another worktree.
        //
        //    ① direct switch (branch exists locally)          ───────────────────
        //    ② otherwise try to track the remote branch       ───────────────────
        //    ③ as last resort create an *orphan* local branch ───────────────────
        let mut ok = Command::new("git")
            .args([
                "switch",
                "--force",
                "--ignore-other-worktrees",
                &orig_branch,
            ]) // <-- --force here
            .status()?
            .success();

        if !ok {
            // remote may exist – create local branch that tracks it
            ok = Command::new("git")
                .args([
                    "switch",
                    "--force", // allow switching to a branch that already exists
                    "--ignore-other-worktrees",
                    "-c",
                    &orig_branch,
                    "--track",
                    &format!("origin/{}", orig_branch),
                ])
                .status()?
                .success();
        }

        if !ok {
            // last resort: create orphaned branch with same name
            let out = Command::new("git")
                .args([
                    "switch",
                    "--force",
                    "--ignore-other-worktrees",
                    "-c",
                    &orig_branch,
                ])
                .output()?;
            if !out.status.success() {
                return Err(format!(
                    "failed to switch to temp branch {orig_branch}: {}",
                    String::from_utf8_lossy(&out.stderr)
                )
                .into());
            }
        }

        // ── 6. replay the dirty state inside the temp work-tree ────────────────

        // 6a. staged patch → index only
        if !staged_patch.is_empty() {
            let mut child = Command::new("git")
                .args(["apply", "--cached", "-"])
                .stdin(std::process::Stdio::piped())
                .spawn()?;
            child.stdin.as_mut().unwrap().write_all(&staged_patch)?;
            if !child.wait()?.success() {
                return Err("failed to apply staged patch".into());
            }
        }

        // 6b. unstaged patch → working tree
        if !unstaged_patch.is_empty() {
            let mut child = Command::new("git")
                .args(["apply", "-"])
                .stdin(std::process::Stdio::piped())
                .spawn()?;
            child.stdin.as_mut().unwrap().write_all(&unstaged_patch)?;
            if !child.wait()?.success() {
                return Err("failed to apply unstaged patch".into());
            }
        }

        // 6c. untracked files
        if !untracked_list.is_empty() {
            for path in untracked_list.split('\0').filter(|p| !p.is_empty()) {
                let from = orig_root.join(path);
                let to = path; // relative inside temp WT

                // Check if source file exists before trying to copy
                // (files might have been renamed/moved and no longer exist at original path)
                if !from.exists() {
                    eprintln!(
                        "Warning: Skipping untracked file copy - source doesn't exist: {}",
                        from.display()
                    );
                    continue;
                }

                if let Some(parent) = Path::new(to).parent() {
                    if let Err(e) = fs_err::create_dir_all(parent) {
                        eprintln!(
                            "Warning: Failed to create directory {}: {}",
                            parent.display(),
                            e
                        );
                        continue;
                    }
                }

                if let Err(e) = fs_err::copy(&from, to) {
                    eprintln!(
                        "Warning: Failed to copy untracked file from {} to {}: {}",
                        from.display(),
                        to,
                        e
                    );
                    continue;
                }
            }
        }

        // All done – temp work-tree now has *exact* dirty state.
        Ok(Self {
            path,
            orig_root,
            orig_branch,
        })
    }

    /// Get the original worktree root path
    pub fn original_root(&self) -> &PathBuf {
        &self.orig_root
    }
}

impl Drop for TempWorktree {
    fn drop(&mut self) {
        // 1. hop back to the original worktree (ignore errors to stay unwind-safe)
        let _ = std::env::set_current_dir(&self.orig_root);

        // 2. if we’re detached, return to the original branch
        let is_detached = Command::new("git")
            .args(["symbolic-ref", "--quiet", "HEAD"])
            .status()
            .map(|s| !s.success())
            .unwrap_or(false);

        if is_detached {
            let _ = Command::new("git")
                .args(["switch", &self.orig_branch])
                .status();
        }

        // 3. remove the temporary worktree
        let _ = Command::new("git")
            .args(["worktree", "remove", "--force", self.path.to_str().unwrap()])
            .status();

        let _ = fs_err::remove_dir_all(&self.path); // belt-and-suspenders
    }
}

/// Check if we're currently in a temporary worktree
pub fn is_in_temp_worktree() -> bool {
    if let Ok(current_dir) = std::env::current_dir() {
        if let Some(dir_name) = current_dir.file_name().and_then(|n| n.to_str()) {
            return dir_name.starts_with("autopr-wt-");
        }
    }
    false
}

/// Clean up old patch files to prevent accumulation
/// Removes patch files older than `days_old` days
pub fn cleanup_old_patches(app: &mut App, days_old: u64) -> Result<(), Box<dyn Error>> {
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

    if !patches_dir.exists() {
        return Ok(());
    }

    let cutoff_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs()
        - (days_old * 24 * 60 * 60);

    let mut cleaned_count = 0;
    for entry in fs_err::read_dir(&patches_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() && path.extension().is_some_and(|ext| ext == "patch") {
            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                // Extract timestamp from filename (format: staged-TIMESTAMP.patch or unstaged-TIMESTAMP.patch)
                if let Some(timestamp_str) =
                    filename.split('-').nth(1).and_then(|s| s.split('.').next())
                {
                    if let Ok(timestamp) = timestamp_str.parse::<u64>() {
                        if timestamp < cutoff_time {
                            if let Err(e) = fs_err::remove_file(&path) {
                                app.add_log(
                                    "WARN",
                                    format!(
                                        "Failed to remove old patch file {}: {}",
                                        path.display(),
                                        e
                                    ),
                                );
                            } else {
                                cleaned_count += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    if cleaned_count > 0 {
        app.add_log(
            "INFO",
            format!("Cleaned up {} old patch files", cleaned_count),
        );
    }

    Ok(())
}

#[cfg(test)]
#[path = "git_temp_worktree/tests.rs"]
mod tests;
