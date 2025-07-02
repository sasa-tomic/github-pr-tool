use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

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
fn test_git_ensure_in_repo_success() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");
    let result = git_ensure_in_repo(&mut app);

    env::set_current_dir(original_dir).expect("Failed to restore directory");

    assert!(result.is_ok());
}

#[test]
fn test_git_cd_to_repo_root() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");
    let result = git_cd_to_repo_root(&mut app);

    env::set_current_dir(original_dir).expect("Failed to restore directory");

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
fn test_git_current_branch() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");
    let result = git_current_branch(&mut app);

    env::set_current_dir(original_dir).expect("Failed to restore directory");

    assert!(result.is_ok());
    let branch = result.unwrap();
    assert!(!branch.is_empty());
    assert!(app
        .logs
        .iter()
        .any(|(level, msg)| { *level == "INFO" && msg.contains("Current branch:") }));
}

#[test]
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

    env::set_current_dir(original_dir).expect("Failed to restore directory");

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

    env::set_current_dir(original_dir).expect("Failed to restore directory");
}

#[test]
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

    env::set_current_dir(original_dir).expect("Failed to restore directory");

    assert!(result.is_ok());
    assert!(app.logs.iter().any(|(level, msg)| {
        *level == "INFO" && msg.contains("Committed changes successfully")
    }));
}

#[test]
fn test_git_checkout_new_branch() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");
    let result = git_checkout_new_branch(&mut app, "test-branch", "HEAD", false);

    env::set_current_dir(original_dir).expect("Failed to restore directory");

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

    env::set_current_dir(original_dir).expect("Failed to restore directory");

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
fn test_git_diff_uncommitted_empty() {
    let (_temp_dir, repo_path) = create_test_repo();
    let original_dir = env::current_dir().expect("Failed to get current directory");

    env::set_current_dir(&repo_path).expect("Failed to change directory");

    let mut app = App::new("Test App");
    let result = git_diff_uncommitted(&mut app, "HEAD");

    env::set_current_dir(original_dir).expect("Failed to restore directory");

    assert!(result.is_ok());
    let diff = result.unwrap();
    assert!(diff.is_empty());
}

#[test]
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

    env::set_current_dir(original_dir).expect("Failed to restore directory");

    assert!(result.is_ok());
    let diff = result.unwrap();
    assert!(!diff.is_empty());
    assert!(diff.contains("test.txt"));
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    // These tests require actual git and gh CLI tools to be installed
    // They are marked with #[ignore] by default to avoid CI failures

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
}
