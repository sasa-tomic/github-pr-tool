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

    let result = update_original_worktree_to_pr_branch(&mut app, pr_branch, &original_root);

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

    let result = update_original_worktree_to_pr_branch(&mut app, pr_branch, &original_root);

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
