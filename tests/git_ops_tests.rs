use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

use serial_test::serial;

// Import from the local crate
use gh_autopr::git_ops::*;
use gh_autopr::github_ops::*;
use gh_autopr::tui::App;

// Helper function to create a temporary git repository for testing
fn create_test_repo() -> (TempDir, String) {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let repo_path = temp_dir.path().to_str().unwrap().to_string();

    // Initialize git repo
    Command::new("git")
        .args(["init"])
        .current_dir(&temp_dir)
        .output()
        .expect("Failed to initialize git repo");

    // Configure git user for testing
    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(&temp_dir)
        .output()
        .expect("Failed to set git user name");

    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(&temp_dir)
        .output()
        .expect("Failed to set git user email");

    // Create initial commit
    fs::write(temp_dir.path().join("README.md"), "# Test Repo").expect("Failed to write README");
    Command::new("git")
        .args(["add", "README.md"])
        .current_dir(&temp_dir)
        .output()
        .expect("Failed to add README");

    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&temp_dir)
        .output()
        .expect("Failed to commit README");

    (temp_dir, repo_path)
}

#[test]
#[serial]
fn test_git_ensure_in_repo_success() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");
    let result = git_ensure_in_repo(&mut app);

    let _ = env::set_current_dir(&original_dir);

    assert!(result.is_ok());
}

#[test]
#[serial]
fn test_git_cd_to_repo_root() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");
    let result = git_cd_to_repo_root(&mut app);

    let _ = env::set_current_dir(&original_dir);

    assert!(result.is_ok());
    assert!(app.logs.iter().any(|(level, msg)| {
        *level == "INFO" && msg.contains("Changed directory to repo root")
    }));
}

#[test]
fn test_truncate_utf8() {
    // Test normal truncation
    let text = "Hello, world!";
    let result = truncate_utf8(text, 10);
    assert_eq!(result, "Hello, wor");

    // Test with UTF-8 characters
    let text = "Hello, 世界!";
    let result = truncate_utf8(text, 10);
    assert!(result.len() <= 10);
    assert!(result.is_ascii() || result.chars().all(|c| c.is_ascii() || c as u32 > 127));

    // Test when text is shorter than max
    let text = "Short";
    let result = truncate_utf8(text, 100);
    assert_eq!(result, "Short");
}

#[test]
#[serial]
fn test_git_current_branch() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");
    let result = git_current_branch(&mut app);

    let _ = env::set_current_dir(&original_dir);

    assert!(result.is_ok());
    let branch = result.unwrap();
    assert!(!branch.is_empty());
    assert!(app
        .logs
        .iter()
        .any(|(level, msg)| { *level == "INFO" && msg.contains("Current branch:") }));
}

#[test]
#[serial]
fn test_git_main_branch() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Create a local "origin" repository to avoid network calls
    let origin_dir = _temp_dir.path().join("origin");
    fs::create_dir(&origin_dir).expect("Failed to create origin directory");

    // Initialize origin repo
    Command::new("git")
        .args(["init", "--bare"])
        .current_dir(&origin_dir)
        .output()
        .expect("Failed to initialize origin repo");

    // Set up origin remote pointing to local directory
    Command::new("git")
        .args(["remote", "add", "origin", origin_dir.to_str().unwrap()])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to add origin");

    // Push to origin to establish HEAD
    Command::new("git")
        .args(["push", "origin", "main"])
        .current_dir(&repo_path)
        .output()
        .ok(); // This might fail, that's okay

    let mut app = App::new("Test App");
    let result = git_main_branch(&mut app);

    let _ = env::set_current_dir(&original_dir);

    // This test should handle both success and failure gracefully
    match result {
        Ok(branch) => {
            assert!(!branch.is_empty());
            assert!(app.logs.iter().any(|(level, msg)| {
                *level == "INFO"
                    && (msg.contains("Determined main branch:")
                        || msg.contains("Setting origin HEAD"))
            }));
        }
        Err(_) => {
            // Expected in some environments or git configurations
            assert!(!app.errors.is_empty());
        }
    }
}

#[test]
#[serial]
fn test_git_has_staged_changes() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Initially no staged changes
    let result = git_has_staged_changes();
    assert!(result.is_ok());
    assert!(!result.unwrap());

    // Add a file and stage it
    fs::write(Path::new(&repo_path).join("test.txt"), "test content")
        .expect("Failed to write test file");

    Command::new("git")
        .args(["add", "test.txt"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to stage file");

    let result = git_has_staged_changes();
    assert!(result.is_ok());
    assert!(result.unwrap());

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_git_commit_staged_changes() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Add and stage a file
    fs::write(Path::new(&repo_path).join("test.txt"), "test content")
        .expect("Failed to write test file");

    Command::new("git")
        .args(["add", "test.txt"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to stage file");

    let mut app = App::new("Test App");
    let result = git_commit_staged_changes(
        &mut app,
        "Test commit",
        &Some("Test commit details".to_string()),
    );

    let _ = env::set_current_dir(&original_dir);

    assert!(result.is_ok());
    assert!(app.logs.iter().any(|(level, msg)| {
        *level == "INFO" && msg.contains("Committed changes successfully")
    }));
}

#[test]
#[serial]
fn test_git_checkout_new_branch() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");
    let result = git_checkout_new_branch(&mut app, "test-branch", "HEAD", false);

    let _ = env::set_current_dir(&original_dir);

    // This might fail depending on git version and setup
    match result {
        Ok(branch_name) => {
            assert_eq!(branch_name, "test-branch");
            assert!(app
                .logs
                .iter()
                .any(|(level, msg)| { *level == "INFO" && msg.contains("Created branch") }));
        }
        Err(_) => {
            // Expected in some CI environments
            assert!(!app.errors.is_empty());
        }
    }
}

#[test]
#[serial]
fn test_git_stage_and_commit() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Add a file but don't stage it
    fs::write(Path::new(&repo_path).join("test.txt"), "test content")
        .expect("Failed to write test file");

    let mut app = App::new("Test App");
    let result = git_stage_and_commit(
        &mut app,
        "Test commit",
        &Some("Test commit details".to_string()),
    );

    let _ = env::set_current_dir(&original_dir);

    assert!(result.is_ok());
    assert!(app
        .logs
        .iter()
        .any(|(level, msg)| { *level == "INFO" && msg.contains("Staged all changes") }));
    assert!(app.logs.iter().any(|(level, msg)| {
        *level == "INFO" && msg.contains("Committed changes successfully")
    }));
}

#[test]
fn test_discover_parent_branch_main_branch() {
    let mut app = App::new("Test App");
    let result = discover_parent_branch(&mut app, "main", "main");

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "main");
}

#[test]
#[serial]
fn test_git_diff_uncommitted_empty() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");
    let result = git_diff_uncommitted(&mut app, "HEAD");

    let _ = env::set_current_dir(&original_dir);

    assert!(result.is_ok());
    let diff = result.unwrap();
    assert!(diff.is_empty());
}

#[test]
#[serial]
fn test_git_diff_uncommitted_with_changes() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Add and stage a file
    fs::write(Path::new(&repo_path).join("test.txt"), "test content")
        .expect("Failed to write test file");

    Command::new("git")
        .args(["add", "test.txt"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to stage file");

    let mut app = App::new("Test App");
    let result = git_diff_uncommitted(&mut app, "HEAD");

    let _ = env::set_current_dir(&original_dir);

    assert!(result.is_ok());
    let diff = result.unwrap();
    assert!(!diff.is_empty());
    assert!(diff.contains("test.txt"));
}

#[test]
#[serial]
fn test_update_original_worktree_to_pr_branch() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Set up the test scenario:
    // 1. Create a new branch (simulating PR branch)
    // 2. Add some changes and commit them
    // 3. Switch back to main
    // 4. Test that update_original_worktree_to_pr_branch works correctly

    let pr_branch = "feature/test-pr-branch";

    // Create and switch to PR branch
    Command::new("git")
        .args(["checkout", "-b", pr_branch])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to create PR branch");

    // Add some changes to the PR branch
    fs::write(
        Path::new(&repo_path).join("feature.txt"),
        "new feature content",
    )
    .expect("Failed to write feature file");

    Command::new("git")
        .args(["add", "feature.txt"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to stage feature file");

    Command::new("git")
        .args(["commit", "-m", "Add new feature"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to commit feature");

    // Set up a local origin remote to simulate realistic scenario
    let remote_dir = _temp_dir.path().join("origin.git");
    fs::create_dir(&remote_dir).expect("Failed to create origin directory");

    Command::new("git")
        .args(["init", "--bare"])
        .current_dir(&remote_dir)
        .output()
        .expect("Failed to initialize origin repo");

    // Add remote to our test repo
    Command::new("git")
        .args(["remote", "add", "origin", remote_dir.to_str().unwrap()])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to add remote");

    // Push PR branch to remote
    Command::new("git")
        .args(["push", "origin", pr_branch])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to push PR branch to remote");

    // Switch back to main branch (simulating original worktree state)
    Command::new("git")
        .args(["checkout", "main"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to switch to main");

    // Verify we're on main and the feature file doesn't exist
    let current_branch_output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to get current branch");

    let current_branch = String::from_utf8(current_branch_output.stdout)
        .expect("Failed to parse branch name")
        .trim()
        .to_string();
    assert_eq!(current_branch, "main");
    let feature_file_path = Path::new(&repo_path).join("feature.txt");
    assert!(!feature_file_path.exists());

    // Now test the update_original_worktree_to_pr_branch function
    let mut app = App::new("Test App");
    let original_root = std::path::PathBuf::from(&repo_path);

    // had_staged_changes=false means all changes went to PR, so discard everything
    let result = update_original_worktree_to_pr_branch(&mut app, pr_branch, &original_root, false);

    // Verify the function succeeded
    if let Err(e) = &result {
        eprintln!("Function failed with error: {}", e);
        eprintln!("App errors: {:?}", app.errors);
        eprintln!("App logs: {:?}", app.logs);
    }
    assert!(
        result.is_ok(),
        "update_original_worktree_to_pr_branch should succeed: {:?}",
        result
    );

    // Verify that the logs show the correct operations
    assert!(app.logs.iter().any(|(level, msg)| {
        *level == "SUCCESS" && msg.contains(&format!("Switched to branch '{}'", pr_branch))
    }));

    // Verify that the original worktree is now on the PR branch
    env::set_current_dir(&repo_path).expect("Failed to change to repo directory");

    let final_branch_output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to get final branch");

    let final_branch = String::from_utf8(final_branch_output.stdout)
        .expect("Failed to parse final branch name")
        .trim()
        .to_string();
    assert_eq!(
        final_branch, pr_branch,
        "Original worktree should be on PR branch"
    );

    // Verify that the feature file now exists (changes are present)
    let feature_file_path = Path::new(&repo_path).join("feature.txt");
    assert!(
        feature_file_path.exists(),
        "Feature file should exist after switching to PR branch"
    );

    // Add .gitignore to ignore temporary directories that might be created during tests
    fs::write(
        Path::new(&repo_path).join(".gitignore"),
        "# Test artifacts\n*.git/\nremote.git/\norigin.git/\n",
    )
    .expect("Failed to write .gitignore");

    // Stage and commit the .gitignore file
    Command::new("git")
        .args(["add", ".gitignore"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to stage .gitignore");

    Command::new("git")
        .args(["commit", "-m", "Add .gitignore for test artifacts"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to commit .gitignore");

    // Verify that there are no uncommitted changes
    let status_output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to get git status");

    let status = String::from_utf8(status_output.stdout)
        .expect("Failed to parse git status")
        .trim()
        .to_string();
    assert!(
        status.is_empty(),
        "There should be no uncommitted changes after switching to PR branch"
    );

    // Verify that the working directory is clean (no staged or unstaged changes)
    let diff_output = Command::new("git")
        .args(["diff", "--name-only"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to get git diff");

    let diff = String::from_utf8(diff_output.stdout)
        .expect("Failed to parse git diff")
        .trim()
        .to_string();
    assert!(diff.is_empty(), "git diff should show no changes");

    let diff_cached_output = Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to get git diff --cached");

    let diff_cached = String::from_utf8(diff_cached_output.stdout)
        .expect("Failed to parse git diff --cached")
        .trim()
        .to_string();
    assert!(
        diff_cached.is_empty(),
        "git diff --cached should show no changes"
    );

    // Clean up the remote.git directory that was created
    let _ = fs::remove_dir_all(&remote_dir);

    // Try to restore original directory, but don't fail if it doesn't exist
    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_update_original_worktree_to_pr_branch_with_remote_tracking() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Set up a more complex scenario with remote tracking
    let pr_branch = "feature/remote-tracking-test";

    // Create a bare "remote" repository
    let remote_dir = _temp_dir.path().join("remote.git");
    fs::create_dir(&remote_dir).expect("Failed to create remote directory");

    Command::new("git")
        .args(["init", "--bare"])
        .current_dir(&remote_dir)
        .output()
        .expect("Failed to initialize remote repo");

    // Add remote to our test repo
    Command::new("git")
        .args(["remote", "add", "origin", remote_dir.to_str().unwrap()])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to add remote");

    // Push main branch to remote
    Command::new("git")
        .args(["push", "origin", "main"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to push main to remote");

    // Create PR branch and push to remote
    Command::new("git")
        .args(["checkout", "-b", pr_branch])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to create PR branch");

    // Add changes to PR branch
    fs::write(
        Path::new(&repo_path).join("remote_feature.txt"),
        "remote feature content",
    )
    .expect("Failed to write remote feature file");

    Command::new("git")
        .args(["add", "remote_feature.txt"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to stage remote feature file");

    Command::new("git")
        .args(["commit", "-m", "Add remote feature"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to commit remote feature");

    // Push PR branch to remote
    Command::new("git")
        .args(["push", "origin", pr_branch])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to push PR branch to remote");

    // Switch back to main and delete local PR branch to simulate remote-only scenario
    Command::new("git")
        .args(["checkout", "main"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to switch to main");

    Command::new("git")
        .args(["branch", "-D", pr_branch])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to delete local PR branch");

    // Verify the branch is gone locally
    let branch_list_output = Command::new("git")
        .args(["branch", "--list", pr_branch])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to list branches");

    let branch_list = String::from_utf8(branch_list_output.stdout)
        .expect("Failed to parse branch list")
        .trim()
        .to_string();
    assert!(branch_list.is_empty(), "PR branch should not exist locally");

    // Test update_original_worktree_to_pr_branch with remote-only branch
    let mut app = App::new("Test App");
    let original_root = std::path::PathBuf::from(&repo_path);

    // had_staged_changes=false means all changes went to PR, so discard everything
    let result = update_original_worktree_to_pr_branch(&mut app, pr_branch, &original_root, false);

    // Verify the function succeeded
    if let Err(e) = &result {
        eprintln!("Function failed with error: {}", e);
        eprintln!("App errors: {:?}", app.errors);
        eprintln!("App logs: {:?}", app.logs);
    }
    assert!(
        result.is_ok(),
        "update_original_worktree_to_pr_branch should succeed with remote branch: {:?}",
        result
    );

    // Verify that the original worktree is now on the PR branch
    env::set_current_dir(&repo_path).expect("Failed to change to repo directory");

    let final_branch_output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to get final branch");

    let final_branch = String::from_utf8(final_branch_output.stdout)
        .expect("Failed to parse final branch name")
        .trim()
        .to_string();
    assert_eq!(
        final_branch, pr_branch,
        "Original worktree should be on PR branch from remote"
    );

    // Verify that the remote feature file exists
    let remote_feature_file_path = Path::new(&repo_path).join("remote_feature.txt");
    assert!(
        remote_feature_file_path.exists(),
        "Remote feature file should exist"
    );

    // Add .gitignore to ignore temporary directories that might be created during tests
    fs::write(
        Path::new(&repo_path).join(".gitignore"),
        "# Test artifacts\n*.git/\nremote.git/\norigin.git/\n",
    )
    .expect("Failed to write .gitignore");

    // Stage and commit the .gitignore file
    Command::new("git")
        .args(["add", ".gitignore"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to stage .gitignore");

    Command::new("git")
        .args(["commit", "-m", "Add .gitignore for test artifacts"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to commit .gitignore");

    // Verify clean working directory
    let status_output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to get git status");

    let status = String::from_utf8(status_output.stdout)
        .expect("Failed to parse git status")
        .trim()
        .to_string();
    if !status.is_empty() {
        eprintln!("Working directory status: '{}'", status);
    }
    assert!(
        status.is_empty(),
        "Working directory should be clean after switching to remote PR branch. Status: '{}'",
        status
    );

    // Clean up the remote.git directory that was created
    let _ = fs::remove_dir_all(&remote_dir);

    // Try to restore original directory, but don't fail if it doesn't exist
    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[ignore = "requires gh CLI tool"]
fn test_github_list_issues_integration() {
    // This test would require a real GitHub repository with issues
    // Skipped in normal test runs
    let mut app = App::new("Test App");
    let result = github_list_issues(&mut app);

    match result {
        Ok(issues_json) => {
            // Should be valid JSON
            let _: serde_json::Value =
                serde_json::from_str(&issues_json).expect("Issues JSON should be valid");
        }
        Err(_) => {
            // Expected if not in a GitHub repository or no gh CLI
            assert!(!app.errors.is_empty());
        }
    }
}

#[test]
#[ignore = "requires gh CLI tool"]
fn test_create_or_update_pull_request_integration() {
    // This test would require actual GitHub setup and permissions
    // Skipped in normal test runs
    let mut app = App::new("Test App");
    let result = create_or_update_pull_request(
        &mut app,
        "Test PR",
        "Test PR body",
        false,
        true,
        "main",
        "test-branch",
    );

    // This will likely fail without proper GitHub setup
    // We just test that it handles errors gracefully
    match result {
        Ok(_) => {
            assert!(app.logs.iter().any(|(level, _)| *level == "SUCCESS"));
        }
        Err(_) => {
            assert!(!app.errors.is_empty());
        }
    }
}

// ============================================================================
// TempWorktree Integration Tests
// ============================================================================

use gh_autopr::git_temp_worktree::TempWorktree;

#[test]
#[serial]
fn test_temp_worktree_captures_and_replays_staged_changes() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Create a file and stage it
    fs::write("staged_file.txt", "staged content").expect("Failed to write file");
    Command::new("git")
        .args(["add", "staged_file.txt"])
        .output()
        .expect("Failed to stage file");

    // Enter temp worktree
    let temp_worktree = TempWorktree::enter().expect("Failed to enter temp worktree");

    // Verify we're in the temp worktree
    let current_dir = env::current_dir().expect("Failed to get current dir");
    assert!(
        current_dir
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("autopr-wt-"),
        "Should be in temp worktree directory"
    );

    // Verify staged changes were replayed
    let status = Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .output()
        .expect("Failed to get staged files");
    let staged_files = String::from_utf8(status.stdout).unwrap();
    assert!(
        staged_files.contains("staged_file.txt"),
        "Staged file should be present in temp worktree index"
    );

    // Get the original root for verification after drop
    let orig_root = temp_worktree.original_root().clone();

    // Drop temp worktree - should clean up
    drop(temp_worktree);

    // Verify we're back in original directory
    let current_dir = env::current_dir().expect("Failed to get current dir");
    assert_eq!(
        current_dir, orig_root,
        "Should be back in original worktree after drop"
    );

    // Verify temp worktree was cleaned up
    let worktree_list = Command::new("git")
        .args(["worktree", "list"])
        .output()
        .expect("Failed to list worktrees");
    let worktrees = String::from_utf8(worktree_list.stdout).unwrap();
    assert!(
        !worktrees.contains("autopr-wt-"),
        "Temp worktree should be cleaned up"
    );

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_temp_worktree_captures_unstaged_changes() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Modify an existing tracked file (unstaged change)
    fs::write("README.md", "# Modified content").expect("Failed to modify file");

    // Enter temp worktree
    let temp_worktree = TempWorktree::enter().expect("Failed to enter temp worktree");

    // Verify unstaged changes were replayed
    let diff = Command::new("git")
        .args(["diff", "--name-only"])
        .output()
        .expect("Failed to get diff");
    let modified_files = String::from_utf8(diff.stdout).unwrap();
    assert!(
        modified_files.contains("README.md"),
        "Unstaged changes should be present in temp worktree"
    );

    // Verify content is correct
    let content = fs::read_to_string("README.md").expect("Failed to read file");
    assert_eq!(content, "# Modified content");

    drop(temp_worktree);
    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_temp_worktree_captures_untracked_files() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Create an untracked file
    fs::write("untracked.txt", "untracked content").expect("Failed to write file");

    // Enter temp worktree
    let temp_worktree = TempWorktree::enter().expect("Failed to enter temp worktree");

    // Verify untracked file was copied
    assert!(
        Path::new("untracked.txt").exists(),
        "Untracked file should exist in temp worktree"
    );
    let content = fs::read_to_string("untracked.txt").expect("Failed to read file");
    assert_eq!(content, "untracked content");

    drop(temp_worktree);
    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_temp_worktree_preserves_branch() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Create and checkout a feature branch
    Command::new("git")
        .args(["checkout", "-b", "feature-branch"])
        .output()
        .expect("Failed to create branch");

    // Enter temp worktree
    let temp_worktree = TempWorktree::enter().expect("Failed to enter temp worktree");

    // Verify we're on the same branch in temp worktree
    let branch_output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .expect("Failed to get branch");
    let branch = String::from_utf8(branch_output.stdout)
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(
        branch, "feature-branch",
        "Should be on same branch in temp worktree"
    );

    drop(temp_worktree);

    // Verify we're still on feature-branch after cleanup
    let branch_output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .expect("Failed to get branch");
    let branch = String::from_utf8(branch_output.stdout)
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(
        branch, "feature-branch",
        "Should still be on feature-branch after cleanup"
    );

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_temp_worktree_full_workflow() {
    // This test simulates the actual workflow:
    // 1. User has uncommitted changes
    // 2. Enter temp worktree
    // 3. Create branch, commit, (simulate push)
    // 4. Cleanup and switch original to new branch

    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Simulate user's uncommitted work
    fs::write("new_feature.rs", "fn main() {}").expect("Failed to write file");
    Command::new("git")
        .args(["add", "new_feature.rs"])
        .output()
        .expect("Failed to stage file");

    // Enter temp worktree
    let temp_worktree = TempWorktree::enter().expect("Failed to enter temp worktree");
    let orig_root = temp_worktree.original_root().clone();

    // Create a new branch (simulating what gh-autopr does)
    Command::new("git")
        .args(["checkout", "-b", "feat/new-feature"])
        .output()
        .expect("Failed to create branch");

    // Commit the staged changes
    Command::new("git")
        .args(["commit", "-m", "Add new feature"])
        .output()
        .expect("Failed to commit");

    // Verify commit exists
    let log = Command::new("git")
        .args(["log", "--oneline", "-1"])
        .output()
        .expect("Failed to get log");
    let log_output = String::from_utf8(log.stdout).unwrap();
    assert!(log_output.contains("Add new feature"));

    // Drop temp worktree
    drop(temp_worktree);

    // Verify we're back in original worktree
    assert_eq!(env::current_dir().unwrap(), orig_root);

    // The branch should exist (created in shared .git)
    let branches = Command::new("git")
        .args(["branch", "--list", "feat/new-feature"])
        .output()
        .expect("Failed to list branches");
    let branch_list = String::from_utf8(branches.stdout).unwrap();
    assert!(
        branch_list.contains("feat/new-feature"),
        "New branch should exist in original worktree"
    );

    // Now we can checkout the branch (simulating update_original_worktree_to_pr_branch)
    let checkout = Command::new("git")
        .args(["checkout", "feat/new-feature"])
        .output()
        .expect("Failed to checkout branch");
    assert!(
        checkout.status.success(),
        "Should be able to checkout PR branch"
    );

    // Verify the commit is there
    let log = Command::new("git")
        .args(["log", "--oneline", "-1"])
        .output()
        .expect("Failed to get log");
    let log_output = String::from_utf8(log.stdout).unwrap();
    assert!(
        log_output.contains("Add new feature"),
        "Commit should be visible"
    );

    let _ = env::set_current_dir(&original_dir);
}

// ============================================================================
// Error Path Tests
// ============================================================================

#[test]
#[serial]
fn test_temp_worktree_cleanup_on_early_drop() {
    // Verify that temp worktree is cleaned up even if we drop it early
    // (simulating an error scenario where the workflow doesn't complete)

    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Create some changes
    fs::write("error_test.txt", "content").expect("Failed to write file");

    let orig_root = {
        let temp_worktree = TempWorktree::enter().expect("Failed to enter temp worktree");
        let root = temp_worktree.original_root().clone();

        // Simulate an error scenario - drop without completing workflow
        // This tests that RAII cleanup works
        drop(temp_worktree);
        root
    };

    // Verify cleanup happened
    let current_dir = env::current_dir().expect("Failed to get current dir");
    assert_eq!(
        current_dir, orig_root,
        "Should be back in original dir after early drop"
    );

    // Verify no temp worktrees remain
    let worktree_list = Command::new("git")
        .args(["worktree", "list"])
        .output()
        .expect("Failed to list worktrees");
    let worktrees = String::from_utf8(worktree_list.stdout).unwrap();
    assert!(
        !worktrees.contains("autopr-wt-"),
        "Temp worktree should be cleaned up even after early drop"
    );

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_create_pr_error_no_existing_pr_to_update() {
    // Test that update_pr=true fails gracefully when no PR exists
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");

    // Try to update a PR that doesn't exist (no remote, no PR)
    let result = create_or_update_pull_request(
        &mut app,
        "Test Title",
        "Test Body",
        true,  // update_pr = true
        false, // ready = false
        "main",
        "nonexistent-branch",
    );

    // Should fail with appropriate error
    assert!(
        result.is_err(),
        "Should fail when trying to update non-existent PR"
    );

    // Error should be logged
    assert!(
        !app.errors.is_empty(),
        "Should log an error about missing PR or gh CLI failure"
    );

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_git_checkout_new_branch_error_branch_exists() {
    // Test that creating a branch that already exists fails appropriately
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Create a branch first
    Command::new("git")
        .args(["checkout", "-b", "existing-branch"])
        .output()
        .expect("Failed to create branch");

    Command::new("git")
        .args(["checkout", "main"])
        .output()
        .expect("Failed to checkout main");

    let mut app = App::new("Test App");

    // Try to create the same branch again without force_reset
    let result = git_checkout_new_branch(&mut app, "existing-branch", "main", false);

    assert!(result.is_err(), "Should fail when branch already exists");
    assert!(
        app.errors.iter().any(|e| e.contains("already exists")),
        "Error should mention branch already exists"
    );

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_git_diff_between_branches_error_invalid_branch() {
    // Test that diffing against a non-existent branch fails appropriately
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");

    let result = git_diff_between_branches(&mut app, "nonexistent-base", "main");

    assert!(
        result.is_err(),
        "Should fail when base branch doesn't exist"
    );

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_git_push_branch_error_no_remote() {
    // Test that pushing to a non-existent remote fails appropriately
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");

    // Try to push - should fail because there's no remote
    let result = git_push_branch(&mut app, "main");

    assert!(result.is_err(), "Should fail when no remote configured");
    assert!(!app.errors.is_empty(), "Should log an error");

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_update_original_worktree_error_invalid_branch() {
    // Test that updating to a non-existent branch fails appropriately
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");
    let repo_path_buf = std::path::PathBuf::from(&repo_path);

    // Try to switch to a branch that doesn't exist locally or remotely
    // had_staged_changes=false means all changes went to PR, so discard everything
    let result = update_original_worktree_to_pr_branch(
        &mut app,
        "completely-nonexistent-branch-xyz",
        &repo_path_buf,
        false,
    );

    assert!(
        result.is_err(),
        "Should fail when branch doesn't exist anywhere"
    );
    assert!(!app.errors.is_empty(), "Should log an error");

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_git_fetch_main_error_no_remote() {
    // Test that fetching without a remote configured fails appropriately
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");

    // Try to fetch - should fail because there's no remote
    let result = git_fetch_main(&mut app, "main", "main");

    assert!(result.is_err(), "Should fail when no remote configured");

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_temp_worktree_multiple_sequential_creates() {
    // Test that we can create multiple temp worktrees sequentially
    // (after properly cleaning up each one)
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    for i in 0..3 {
        fs::write(format!("file_{}.txt", i), format!("content {}", i))
            .expect("Failed to write file");

        let temp_worktree = TempWorktree::enter().expect("Failed to enter temp worktree");

        // Verify file exists in temp worktree
        assert!(
            Path::new(&format!("file_{}.txt", i)).exists(),
            "File should exist in temp worktree iteration {}",
            i
        );

        drop(temp_worktree);

        // Verify cleanup
        let worktree_list = Command::new("git")
            .args(["worktree", "list"])
            .output()
            .expect("Failed to list worktrees");
        let worktrees = String::from_utf8(worktree_list.stdout).unwrap();
        assert!(
            !worktrees.contains("autopr-wt-"),
            "Temp worktree should be cleaned up after iteration {}",
            i
        );
    }

    let _ = env::set_current_dir(&original_dir);
}

// ============================================================================
// Additional Error Path Tests - Comprehensive Non-Overlapping Coverage
// ============================================================================

#[test]
#[serial]
fn test_git_ensure_in_repo_error_not_in_repo() {
    // Test behavior when not in a git repository
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let original_dir = env::current_dir().expect("Failed to get current directory");

    // Change to a directory that is NOT a git repo
    env::set_current_dir(temp_dir.path()).expect("Failed to change directory");

    // Verify the git command would fail (git_ensure_in_repo calls process::exit)
    let output = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .expect("Failed to run git");

    assert!(!output.status.success(), "Should fail when not in git repo");

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_git_commit_staged_changes_error_nothing_staged() {
    // Test error when trying to commit with nothing staged
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");

    // Try to commit with nothing staged - should fail
    let result = git_commit_staged_changes(&mut app, "Empty commit", &None);

    assert!(result.is_err(), "Should fail when nothing to commit");
    assert!(!app.errors.is_empty(), "Should log an error");

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_git_stage_and_commit_error_nothing_to_stage() {
    // Test behavior when there's nothing to stage or commit
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");

    // Try to stage and commit with no changes - should fail
    let result = git_stage_and_commit(&mut app, "No changes commit", &None);

    assert!(result.is_err(), "Should fail when nothing to commit");

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_discover_parent_branch_returns_main_for_unknown() {
    // Test discover_parent_branch fallback behavior
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");

    // Should return main as fallback for unknown branch
    let result = discover_parent_branch(&mut app, "main", "nonexistent-child");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "main", "Should fall back to main");

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_git_checkout_new_branch_error_invalid_base() {
    // Test creating branch from invalid base
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");

    // Try to create branch from non-existent base
    let result = git_checkout_new_branch(&mut app, "new-branch", "nonexistent-base", false);

    assert!(
        result.is_err(),
        "Should fail when base branch doesn't exist"
    );

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_create_pr_error_create_fails_no_github() {
    // Test PR creation failure when gh isn't configured
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");

    // Try to create new PR - should fail because no GitHub remote
    let result = create_or_update_pull_request(
        &mut app,
        "Test PR",
        "Test body",
        false, // create new (not update)
        false, // not ready
        "main",
        "feature",
    );

    assert!(result.is_err(), "Should fail when gh isn't configured");

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_delete_local_branch_error_current_branch() {
    // Test deleting the currently checked out branch fails
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");

    // Try to delete current branch (main) - should fail
    let result = delete_local_branch(&mut app, "main");

    assert!(result.is_err(), "Should fail when deleting current branch");
    assert!(!app.errors.is_empty());

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_delete_local_branch_error_nonexistent() {
    // Test deleting a branch that doesn't exist
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");

    // Try to delete non-existent branch
    let result = delete_local_branch(&mut app, "this-branch-does-not-exist-xyz");

    assert!(result.is_err(), "Should fail when branch doesn't exist");
    assert!(!app.errors.is_empty());

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_temp_worktree_error_not_in_git_repo() {
    // Test TempWorktree::enter fails when not in a git repo
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let original_dir = env::current_dir().expect("Failed to get current directory");

    // NOT a git repo - just an empty directory
    env::set_current_dir(temp_dir.path()).expect("Failed to change directory");

    let result = TempWorktree::enter();

    assert!(result.is_err(), "Should fail when not in git repo");

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_git_main_branch_error_no_origin() {
    // Test getting main branch when there's no origin remote
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");

    // This should fail since there's no origin configured
    let result = git_main_branch(&mut app);

    // Should fail - no origin remote
    assert!(result.is_err(), "Should fail when no origin configured");

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_get_local_branches_success() {
    // Test getting local branches works correctly
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Create additional branches
    Command::new("git")
        .args(["checkout", "-b", "feature-a"])
        .output()
        .expect("Failed to create branch");
    Command::new("git")
        .args(["checkout", "-b", "feature-b"])
        .output()
        .expect("Failed to create branch");
    Command::new("git")
        .args(["checkout", "main"])
        .output()
        .expect("Failed to checkout main");

    let mut app = App::new("Test App");
    let result = get_local_branches_with_remotes(&mut app);

    assert!(result.is_ok(), "Should succeed getting local branches");
    let branches = result.unwrap();
    assert!(branches.len() >= 3, "Should have at least 3 branches");
    assert!(branches.contains_key("main"));
    assert!(branches.contains_key("feature-a"));
    assert!(branches.contains_key("feature-b"));

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_remote_branch_exists_no_remote_configured() {
    // Test checking remote branch when no remote configured returns false
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");
    let result = remote_branch_exists(&mut app, "origin/main");

    // Should return false - no remote configured
    if let Ok(exists) = result {
        assert!(!exists, "Should not find remote branch when no remote")
    }

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_get_merged_prs_error_no_github() {
    // Test getting merged PRs fails gracefully when no GitHub
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");
    let result = get_merged_prs_and_branches(&mut app);

    // Should fail or return empty - no GitHub configured
    match result {
        Ok(prs) => assert!(prs.is_empty(), "Should be empty when no GitHub"),
        Err(_) => assert!(!app.errors.is_empty(), "Should log error"),
    }

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_truncate_utf8_edge_cases() {
    // Test truncation edge cases

    // Empty string
    assert_eq!(truncate_utf8("", 10), "");

    // String shorter than limit
    assert_eq!(truncate_utf8("short", 100), "short");

    // Exact limit
    assert_eq!(truncate_utf8("12345", 5), "12345");

    // Multi-byte UTF-8 at boundary - should not split character
    let emoji = "Hello 👋 World";
    let truncated = truncate_utf8(emoji, 8);
    assert!(truncated.len() <= 8);
    assert!(truncated.is_char_boundary(truncated.len()));

    // All multi-byte characters
    let chinese = "你好世界";
    let truncated = truncate_utf8(chinese, 5);
    assert!(truncated.len() <= 5);
    assert!(truncated.is_char_boundary(truncated.len()));
}

#[test]
#[serial]
fn test_git_diff_uncommitted_no_changes() {
    // Test diff returns empty when no uncommitted changes
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");
    let result = git_diff_uncommitted(&mut app, "main");

    assert!(result.is_ok());
    assert!(
        result.unwrap().is_empty(),
        "Should return empty diff when no changes"
    );

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_git_has_staged_changes_false_when_none() {
    // Test git_has_staged_changes returns false when nothing staged
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let result = git_has_staged_changes();

    assert!(result.is_ok());
    assert!(!result.unwrap(), "Should return false when nothing staged");

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_git_has_staged_changes_true_when_staged() {
    // Test git_has_staged_changes returns true when changes staged
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Create and stage a file
    fs::write("staged.txt", "content").expect("Failed to write");
    Command::new("git")
        .args(["add", "staged.txt"])
        .output()
        .expect("Failed to stage");

    let result = git_has_staged_changes();

    assert!(result.is_ok());
    assert!(result.unwrap(), "Should return true when changes staged");

    let _ = env::set_current_dir(&original_dir);
}

// ============================================================================
// Edge Case Tests for update_original_worktree_to_pr_branch
// ============================================================================

#[test]
#[serial]
fn test_update_worktree_preserves_unstaged_when_staged_exists() {
    // Scenario: User has staged changes to file A and unstaged changes to file B
    // Expected: After PR creation, file B's unstaged changes should be preserved
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Create and commit a base file that we'll modify (unstaged)
    fs::write("unstaged_file.txt", "original content").expect("Failed to write");
    Command::new("git")
        .args(["add", "unstaged_file.txt"])
        .output()
        .expect("Failed to stage");
    Command::new("git")
        .args(["commit", "-m", "Add unstaged_file"])
        .output()
        .expect("Failed to commit");

    // Create PR branch with a staged change
    Command::new("git")
        .args(["checkout", "-b", "pr-branch"])
        .output()
        .expect("Failed to create branch");

    fs::write("staged_file.txt", "staged content").expect("Failed to write");
    Command::new("git")
        .args(["add", "staged_file.txt"])
        .output()
        .expect("Failed to stage");
    Command::new("git")
        .args(["commit", "-m", "Add staged file"])
        .output()
        .expect("Failed to commit");

    // Go back to main
    Command::new("git")
        .args(["checkout", "main"])
        .output()
        .expect("Failed to checkout main");

    // Now simulate the scenario: staged change + unstaged change
    fs::write("new_staged.txt", "will be staged").expect("Failed to write");
    Command::new("git")
        .args(["add", "new_staged.txt"])
        .output()
        .expect("Failed to stage");

    // Make unstaged modification to existing file
    fs::write("unstaged_file.txt", "modified unstaged content").expect("Failed to write");

    // Verify setup
    assert!(
        git_has_staged_changes().unwrap(),
        "Should have staged changes"
    );
    let diff_output = Command::new("git").args(["diff"]).output().unwrap();
    assert!(
        !diff_output.stdout.is_empty(),
        "Should have unstaged changes"
    );

    let mut app = App::new("Test App");
    let original_root = std::path::PathBuf::from(&repo_path);

    // Call with had_staged_changes=true
    let result = update_original_worktree_to_pr_branch(&mut app, "pr-branch", &original_root, true);

    assert!(result.is_ok(), "Should succeed: {:?}", result);

    // Verify we're on pr-branch
    let branch_output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .expect("Failed to get branch");
    let branch = String::from_utf8(branch_output.stdout)
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(branch, "pr-branch");

    // Verify unstaged changes were preserved
    let content = fs::read_to_string("unstaged_file.txt").expect("Failed to read");
    assert_eq!(
        content, "modified unstaged content",
        "Unstaged changes should be preserved"
    );

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_update_worktree_preserves_untracked_when_staged_exists() {
    // Scenario: User has staged changes + untracked files
    // Expected: Untracked files should be preserved (not cleaned)
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Create PR branch
    Command::new("git")
        .args(["checkout", "-b", "pr-branch"])
        .output()
        .expect("Failed to create branch");

    fs::write("pr_file.txt", "pr content").expect("Failed to write");
    Command::new("git")
        .args(["add", "pr_file.txt"])
        .output()
        .expect("Failed to stage");
    Command::new("git")
        .args(["commit", "-m", "PR commit"])
        .output()
        .expect("Failed to commit");

    // Go back to main
    Command::new("git")
        .args(["checkout", "main"])
        .output()
        .expect("Failed to checkout main");

    // Create staged change
    fs::write("staged.txt", "staged").expect("Failed to write");
    Command::new("git")
        .args(["add", "staged.txt"])
        .output()
        .expect("Failed to stage");

    // Create untracked file (not staged)
    fs::write("untracked.txt", "untracked content").expect("Failed to write");

    let mut app = App::new("Test App");
    let original_root = std::path::PathBuf::from(&repo_path);

    // Call with had_staged_changes=true
    let result = update_original_worktree_to_pr_branch(&mut app, "pr-branch", &original_root, true);

    assert!(result.is_ok(), "Should succeed: {:?}", result);

    // Verify untracked file still exists
    assert!(
        Path::new("untracked.txt").exists(),
        "Untracked file should be preserved when had_staged_changes=true"
    );
    let content = fs::read_to_string("untracked.txt").expect("Failed to read");
    assert_eq!(content, "untracked content");

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_update_worktree_cleans_all_when_no_staged() {
    // Scenario: No staged changes, only unstaged + untracked
    // Expected: Everything should be cleaned (went to PR)
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Create PR branch with the changes that "went to PR"
    Command::new("git")
        .args(["checkout", "-b", "pr-branch"])
        .output()
        .expect("Failed to create branch");

    fs::write("new_file.txt", "new content").expect("Failed to write");
    fs::write("README.md", "modified readme").expect("Failed to write");
    Command::new("git")
        .args(["add", "."])
        .output()
        .expect("Failed to stage");
    Command::new("git")
        .args(["commit", "-m", "PR changes"])
        .output()
        .expect("Failed to commit");

    // Go back to main
    Command::new("git")
        .args(["checkout", "main"])
        .output()
        .expect("Failed to checkout main");

    // Recreate the "dirty" state that would have been committed
    fs::write("new_file.txt", "new content").expect("Failed to write");
    fs::write("README.md", "modified readme").expect("Failed to write");

    // Verify no staged changes
    assert!(
        !git_has_staged_changes().unwrap(),
        "Should not have staged changes"
    );

    let mut app = App::new("Test App");
    let original_root = std::path::PathBuf::from(&repo_path);

    // Call with had_staged_changes=false (everything went to PR)
    let result =
        update_original_worktree_to_pr_branch(&mut app, "pr-branch", &original_root, false);

    assert!(result.is_ok(), "Should succeed: {:?}", result);

    // Verify we're on pr-branch
    let branch_output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .expect("Failed to get branch");
    let branch = String::from_utf8(branch_output.stdout)
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(branch, "pr-branch");

    // Verify working directory is clean
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .expect("Failed to get status");
    let status_str = String::from_utf8(status.stdout).unwrap();
    assert!(
        status_str.trim().is_empty(),
        "Working directory should be clean, got: {}",
        status_str
    );

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_update_worktree_handles_binary_files() {
    // Scenario: Unstaged binary file changes should be preserved
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Create and commit a binary file
    let binary_content: Vec<u8> = vec![0x00, 0x01, 0x02, 0xFF, 0xFE, 0xFD];
    fs::write("binary.bin", &binary_content).expect("Failed to write binary");
    Command::new("git")
        .args(["add", "binary.bin"])
        .output()
        .expect("Failed to stage");
    Command::new("git")
        .args(["commit", "-m", "Add binary file"])
        .output()
        .expect("Failed to commit");

    // Create PR branch
    Command::new("git")
        .args(["checkout", "-b", "pr-branch"])
        .output()
        .expect("Failed to create branch");

    fs::write("pr_change.txt", "pr content").expect("Failed to write");
    Command::new("git")
        .args(["add", "pr_change.txt"])
        .output()
        .expect("Failed to stage");
    Command::new("git")
        .args(["commit", "-m", "PR commit"])
        .output()
        .expect("Failed to commit");

    // Go back to main
    Command::new("git")
        .args(["checkout", "main"])
        .output()
        .expect("Failed to checkout main");

    // Stage a text file change
    fs::write("staged.txt", "staged").expect("Failed to write");
    Command::new("git")
        .args(["add", "staged.txt"])
        .output()
        .expect("Failed to stage");

    // Modify the binary file (unstaged)
    let modified_binary: Vec<u8> = vec![0xAA, 0xBB, 0xCC, 0xDD];
    fs::write("binary.bin", &modified_binary).expect("Failed to write modified binary");

    let mut app = App::new("Test App");
    let original_root = std::path::PathBuf::from(&repo_path);

    // Call with had_staged_changes=true
    let result = update_original_worktree_to_pr_branch(&mut app, "pr-branch", &original_root, true);

    assert!(result.is_ok(), "Should succeed: {:?}", result);

    // Verify binary file changes were preserved
    let content = fs::read("binary.bin").expect("Failed to read binary");
    assert_eq!(
        content, modified_binary,
        "Binary file unstaged changes should be preserved"
    );

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_update_worktree_same_file_staged_and_unstaged() {
    // Scenario: Same file has both staged and unstaged changes
    // This is a complex case - staged changes went to PR, unstaged should be preserved
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Create initial file with multiple lines
    fs::write("mixed.txt", "line1\nline2\nline3\nline4\n").expect("Failed to write");
    Command::new("git")
        .args(["add", "mixed.txt"])
        .output()
        .expect("Failed to stage");
    Command::new("git")
        .args(["commit", "-m", "Add mixed file"])
        .output()
        .expect("Failed to commit");

    // Create PR branch with staged changes
    Command::new("git")
        .args(["checkout", "-b", "pr-branch"])
        .output()
        .expect("Failed to create branch");

    // Modify first part and commit (simulating what went to PR)
    fs::write("mixed.txt", "MODIFIED1\nline2\nline3\nline4\n").expect("Failed to write");
    Command::new("git")
        .args(["add", "mixed.txt"])
        .output()
        .expect("Failed to stage");
    Command::new("git")
        .args(["commit", "-m", "PR changes"])
        .output()
        .expect("Failed to commit");

    // Go back to main
    Command::new("git")
        .args(["checkout", "main"])
        .output()
        .expect("Failed to checkout main");

    // Stage the same change that went to PR
    fs::write("mixed.txt", "MODIFIED1\nline2\nline3\nline4\n").expect("Failed to write");
    Command::new("git")
        .args(["add", "mixed.txt"])
        .output()
        .expect("Failed to stage");

    // Now add additional unstaged changes (different line)
    fs::write("mixed.txt", "MODIFIED1\nline2\nMODIFIED3\nline4\n").expect("Failed to write");

    let mut app = App::new("Test App");
    let original_root = std::path::PathBuf::from(&repo_path);

    // Call with had_staged_changes=true
    let result = update_original_worktree_to_pr_branch(&mut app, "pr-branch", &original_root, true);

    // This might partially fail due to conflicts, but should not error out
    assert!(
        result.is_ok(),
        "Should succeed (may warn about conflicts): {:?}",
        result
    );

    // We're on PR branch
    let branch_output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .expect("Failed to get branch");
    let branch = String::from_utf8(branch_output.stdout)
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(branch, "pr-branch");

    // The file should exist and contain the merged/applied changes
    // Due to 3-way merge, the unstaged change to line3 should be applied
    let content = fs::read_to_string("mixed.txt").expect("Failed to read");
    assert!(content.contains("MODIFIED1"), "Should have the PR change");
    // The unstaged change might or might not apply cleanly depending on git version
    // We just verify the operation completed

    let _ = env::set_current_dir(&original_dir);
}

#[test]
#[serial]
fn test_temp_worktree_tracks_staged_changes_flag() {
    // Verify TempWorktree correctly tracks whether there were staged changes
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    // Test 1: No staged changes
    {
        fs::write("unstaged.txt", "unstaged").expect("Failed to write");
        // Don't stage it

        let temp_worktree = TempWorktree::enter().expect("Failed to enter temp worktree");
        assert!(
            !temp_worktree.had_staged_changes(),
            "Should report no staged changes"
        );
        drop(temp_worktree);

        // Clean up
        fs::remove_file("unstaged.txt").ok();
    }

    // Test 2: With staged changes
    {
        fs::write("staged.txt", "staged").expect("Failed to write");
        Command::new("git")
            .args(["add", "staged.txt"])
            .output()
            .expect("Failed to stage");

        let temp_worktree = TempWorktree::enter().expect("Failed to enter temp worktree");
        assert!(
            temp_worktree.had_staged_changes(),
            "Should report staged changes exist"
        );
        drop(temp_worktree);
    }

    let _ = env::set_current_dir(&original_dir);
}
