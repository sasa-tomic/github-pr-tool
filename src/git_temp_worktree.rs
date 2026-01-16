use std::error::Error;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// RAII guard for the temp worktree
pub struct TempWorktree {
    path: PathBuf,
    orig_root: PathBuf,
    orig_branch: String,
    /// Whether the original worktree had staged changes.
    /// This determines cleanup behavior: if true, only staged changes went to PR,
    /// so unstaged changes should be preserved in the original worktree.
    had_staged_changes: bool,
}

impl TempWorktree {
    /// Enter a detached worktree that lives in `.git/autopr-wt-<timestamp>`.
    /// Captures all dirty state (staged, unstaged, untracked) and replays it in the temp worktree.
    pub fn enter() -> Result<Self, Box<dyn Error>> {
        // 1. Capture original location and branch
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

        let git_dir = PathBuf::from(
            String::from_utf8(
                Command::new("git")
                    .args(["rev-parse", "--git-dir"])
                    .output()?
                    .stdout,
            )?
            .trim(),
        );

        // 2. Capture dirty state
        let staged_patch = Command::new("git")
            .args(["diff", "--staged", "--binary"])
            .output()?
            .stdout;
        let had_staged_changes = !staged_patch.is_empty();

        let unstaged_patch = {
            let has_unstaged = !Command::new("git")
                .args(["diff", "--quiet"])
                .status()?
                .success();
            if has_unstaged {
                Command::new("git")
                    .args(["diff", "--binary"])
                    .output()?
                    .stdout
            } else {
                Vec::new()
            }
        };

        let untracked_list = String::from_utf8(
            Command::new("git")
                .args(["ls-files", "--others", "--exclude-standard", "-z"])
                .output()?
                .stdout,
        )?;

        // 3. Create temp worktree path inside .git
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
            had_staged_changes,
        })
    }

    /// Get the original worktree root path
    pub fn original_root(&self) -> &PathBuf {
        &self.orig_root
    }

    /// Whether the original worktree had staged changes when entering.
    /// Used to determine cleanup behavior.
    pub fn had_staged_changes(&self) -> bool {
        self.had_staged_changes
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

#[cfg(test)]
#[path = "git_temp_worktree/tests.rs"]
mod tests;
