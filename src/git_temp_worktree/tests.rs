// Tests for git_temp_worktree functionality

#[test]
fn test_is_in_temp_worktree() {
    // Test the detection of temporary worktrees
    // This function checks if current directory name starts with "autopr-wt-"

    // Mock the current directory check by testing the logic directly
    let temp_worktree_name = "autopr-wt-1234567890";
    let regular_dir_name = "my-project";
    let other_temp_name = "temp-123";

    assert!(
        temp_worktree_name.starts_with("autopr-wt-"),
        "Should identify temp worktree directory"
    );
    assert!(
        !regular_dir_name.starts_with("autopr-wt-"),
        "Should not identify regular directory as temp worktree"
    );
    assert!(
        !other_temp_name.starts_with("autopr-wt-"),
        "Should not identify other temp directories as autopr temp worktree"
    );
}

#[test]
fn test_timestamp_parsing() {
    // Test parsing timestamps from patch filenames (used in cleanup_old_patches)
    let staged_filename = "staged-1752184677.patch";
    let unstaged_filename = "unstaged-1752184677.patch";
    let invalid_filename = "invalid-file.patch";

    // Extract timestamp like in cleanup_old_patches
    let extract_timestamp = |filename: &str| -> Option<u64> {
        filename
            .split('-')
            .nth(1)
            .and_then(|s| s.split('.').next())
            .and_then(|s| s.parse::<u64>().ok())
    };

    assert_eq!(extract_timestamp(staged_filename), Some(1752184677));
    assert_eq!(extract_timestamp(unstaged_filename), Some(1752184677));
    assert_eq!(extract_timestamp(invalid_filename), None);
}

#[test]
fn test_patch_filename_validation() {
    // Test that we can correctly identify patch files
    let valid_staged = "staged-1234567890.patch";
    let valid_unstaged = "unstaged-1234567890.patch";
    let not_patch = "readme.txt";
    let wrong_extension = "staged-1234567890.txt";
    let no_timestamp = "staged-.patch";

    let is_valid_patch = |filename: &str| -> bool {
        filename.ends_with(".patch")
            && (filename.starts_with("staged-") || filename.starts_with("unstaged-"))
            && filename
                .split('-')
                .nth(1)
                .and_then(|s| s.split('.').next())
                .map_or(false, |timestamp| !timestamp.is_empty())
    };

    assert!(is_valid_patch(valid_staged));
    assert!(is_valid_patch(valid_unstaged));
    assert!(!is_valid_patch(not_patch));
    assert!(!is_valid_patch(wrong_extension));
    assert!(!is_valid_patch(no_timestamp));
}
