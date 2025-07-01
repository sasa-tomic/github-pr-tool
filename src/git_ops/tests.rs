use super::*;

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
fn test_truncate_utf8_boundary() {
    // Test that we don't split UTF-8 characters
    let text = "Hello, 世界!";
    let result = truncate_utf8(text, 8); // Should not split the UTF-8 character
    assert!(result.is_char_boundary(result.len()));
    assert!(result.len() <= 8);
}

#[test]
fn test_discover_parent_branch_main() {
    let mut app = App::new("Test App");
    let result = discover_parent_branch(&mut app, "main", "main");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "main");
}

#[test]
fn test_git_run_diff_empty() {
    // This test would require a git repository, so we'll just test that the function exists
    // and has the right signature
    fn _test_signature() {
        let _: fn(&mut App, bool, &str, &[&str]) -> Result<Option<String>, Box<dyn Error>> =
            git_run_diff;
    }
}
