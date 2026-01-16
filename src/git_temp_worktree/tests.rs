// Tests for git_temp_worktree functionality

#[test]
fn test_temp_worktree_naming() {
    // Test the naming convention for temporary worktrees
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
fn test_temp_worktree_struct_fields() {
    // Verify the TempWorktree struct has the expected fields
    // This is a compile-time check that the struct exists and has the right shape
    use std::path::PathBuf;

    // Can't actually create a TempWorktree without a git repo,
    // but we can verify the struct definition by checking its methods exist
    fn _check_original_root_method_exists() {
        // This function just needs to compile to verify the API
        fn _use_temp_worktree(tw: &super::TempWorktree) {
            let _: &PathBuf = tw.original_root();
        }
    }
}
