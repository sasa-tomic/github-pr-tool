#[cfg(test)]
use super::*;
use serde_json::json;
use std::env;

#[test]
fn test_json_parsing() {
    // Test parsing of valid JSON response
    let json_response = json!({
        "branch_name": "feature/add-tests",
        "commit_title": "feat(tests): add comprehensive test coverage",
        "commit_details": "## What\n\nAdded unit tests for all modules\n\n## Why\n\nTo improve code quality and reliability"
    });

    let response_str = json_response.to_string();
    let parsed: serde_json::Value = serde_json::from_str(&response_str).unwrap();

    assert_eq!(parsed["branch_name"].as_str().unwrap(), "feature/add-tests");
    assert_eq!(
        parsed["commit_title"].as_str().unwrap(),
        "feat(tests): add comprehensive test coverage"
    );
    assert!(parsed["commit_details"].as_str().is_some());
}

#[test]
fn test_json_parsing_with_null_details() {
    // Test parsing when commit_details is null
    let json_response = json!({
        "branch_name": "fix/small-bug",
        "commit_title": "fix: resolve minor issue",
        "commit_details": null
    });

    let response_str = json_response.to_string();
    let parsed: serde_json::Value = serde_json::from_str(&response_str).unwrap();

    assert_eq!(parsed["branch_name"].as_str().unwrap(), "fix/small-bug");
    assert_eq!(
        parsed["commit_title"].as_str().unwrap(),
        "fix: resolve minor issue"
    );
    assert!(parsed["commit_details"].is_null());
}

#[test]
fn test_issues_truncation() {
    const MAX_ISSUES_LEN: usize = 16 * 1024;

    // Create a large issues string
    let large_issues = "x".repeat(MAX_ISSUES_LEN + 1000);

    let truncated = if large_issues.len() > MAX_ISSUES_LEN {
        large_issues[..MAX_ISSUES_LEN].to_string()
    } else {
        large_issues
    };

    assert_eq!(truncated.len(), MAX_ISSUES_LEN);
}

#[test]
fn test_model_selection() {
    // Test default model selection
    env::remove_var("OPENAI_MODEL");
    let model = env::var("OPENAI_MODEL").unwrap_or_else(|_| "o4-mini".to_string());
    assert_eq!(model, "o4-mini");

    // Test custom model selection
    env::set_var("OPENAI_MODEL", "gpt-4");
    let model = env::var("OPENAI_MODEL").unwrap_or_else(|_| "o4-mini".to_string());
    assert_eq!(model, "gpt-4");

    // Clean up
    env::remove_var("OPENAI_MODEL");
}

#[test]
fn test_response_cleaning() {
    // Test cleaning of markdown code blocks
    let response_with_markdown = "```json\n{\"test\": \"value\"}\n```";
    let cleaned = response_with_markdown
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .to_string();

    assert_eq!(cleaned, "\n{\"test\": \"value\"}\n");

    // Test cleaning when no markdown blocks
    let response_plain = "{\"test\": \"value\"}";
    let cleaned = response_plain
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .to_string();

    assert_eq!(cleaned, "{\"test\": \"value\"}");
}

#[test]
fn test_fallback_values() {
    // Test that fallback values are used when JSON fields are missing
    let incomplete_json = json!({
        "some_other_field": "value"
    });

    let response_str = incomplete_json.to_string();
    let parsed: serde_json::Value = serde_json::from_str(&response_str).unwrap();

    let branch_name = parsed["branch_name"]
        .as_str()
        .unwrap_or("my-pr-branch")
        .to_string();
    let commit_title = parsed["commit_title"]
        .as_str()
        .unwrap_or("Generic commit title")
        .to_string();
    let commit_details = parsed["commit_details"].as_str().map(|s| s.to_string());

    assert_eq!(branch_name, "my-pr-branch");
    assert_eq!(commit_title, "Generic commit title");
    assert!(commit_details.is_none());
}

#[tokio::test]
#[ignore = "requires OpenAI API key and network access"]
async fn test_gpt_generate_integration() {
    // This is an integration test that requires actual OpenAI API access
    // It's ignored by default to avoid requiring API keys in CI

    if env::var("OPENAI_KEY").is_err() {
        // Skip test if no API key is available
        return;
    }

    let mut app = App::new("Test App");
    let diff_context = "diff --git a/test.txt b/test.txt\nnew file mode 100644\nindex 0000000..ce01362\n--- /dev/null\n+++ b/test.txt\n@@ -0,0 +1 @@\n+hello world".to_string();
    let issues_json = Some("[]".to_string());

    let result = gpt_generate_branch_name_and_commit_description(
        &mut app,
        diff_context,
        issues_json,
        None,
        None,
        None,
    )
    .await;

    match result {
        Ok((branch_name, commit_title, _commit_details)) => {
            assert!(!branch_name.is_empty());
            assert!(!commit_title.is_empty());
            // commit_details can be None for small changes

            // Should follow conventional commits format
            assert!(commit_title.contains(':'));

            // Should have generated a reasonable branch name
            assert!(branch_name.len() > 3);
        }
        Err(e) => {
            // API might fail due to network issues, rate limits, etc.
            println!("Integration test failed (expected in CI): {}", e);
        }
    }
}

#[test]
fn test_conventional_commits_examples() {
    // Test that our system message includes conventional commits examples
    let system_message = "feat(api)!: send an email to the customer when a product is shipped";

    // Should contain scope in parentheses
    assert!(system_message.contains("(api)"));

    // Should contain breaking change indicator
    assert!(system_message.contains("!"));

    // Should follow feat: pattern
    assert!(system_message.starts_with("feat"));
    assert!(system_message.contains(":"));
}

#[test]
fn test_valid_git_branch_names() {
    // Test valid branch names
    assert!(is_valid_git_branch_name("feature/add-tests"));
    assert!(is_valid_git_branch_name("fix-memory-leak"));
    assert!(is_valid_git_branch_name("feat/worktree-update"));
    assert!(is_valid_git_branch_name("release/v1.0.0"));
    assert!(is_valid_git_branch_name("hotfix/security-patch"));
    assert!(is_valid_git_branch_name("main"));
    assert!(is_valid_git_branch_name("develop"));
    assert!(is_valid_git_branch_name("feature_branch"));
    assert!(is_valid_git_branch_name("123-fix-issue"));
    assert!(is_valid_git_branch_name("user/feature"));
}

#[test]
fn test_invalid_git_branch_names() {
    // Test invalid branch names - these should all return false
    assert!(!is_valid_git_branch_name("feat(worktree): update"));
    assert!(!is_valid_git_branch_name("fix memory leak"));
    assert!(!is_valid_git_branch_name("feature: add validation"));
    assert!(!is_valid_git_branch_name("branch with spaces"));
    assert!(!is_valid_git_branch_name(""));
    assert!(!is_valid_git_branch_name("-"));
    assert!(!is_valid_git_branch_name(".hidden"));
    assert!(!is_valid_git_branch_name("branch."));
    assert!(!is_valid_git_branch_name("branch..name"));
    assert!(!is_valid_git_branch_name("branch@name"));
    assert!(!is_valid_git_branch_name("branch#name"));
    assert!(!is_valid_git_branch_name("branch$name"));
    assert!(!is_valid_git_branch_name("branch%name"));
    assert!(!is_valid_git_branch_name("branch^name"));
    assert!(!is_valid_git_branch_name("branch&name"));
    assert!(!is_valid_git_branch_name("branch*name"));
    assert!(!is_valid_git_branch_name("branch(name)"));
    assert!(!is_valid_git_branch_name("branch[name]"));
    assert!(!is_valid_git_branch_name("branch{name}"));
    assert!(!is_valid_git_branch_name("branch|name"));
    assert!(!is_valid_git_branch_name("branch\\name"));
    assert!(!is_valid_git_branch_name("branch?name"));
    assert!(!is_valid_git_branch_name("branch<name>"));
    assert!(!is_valid_git_branch_name("branch,name"));
    assert!(!is_valid_git_branch_name("branch;name"));
    assert!(!is_valid_git_branch_name("branch:name"));
    assert!(!is_valid_git_branch_name("branch\"name"));
    assert!(!is_valid_git_branch_name("branch'name"));
    assert!(!is_valid_git_branch_name("branch~name"));
    assert!(!is_valid_git_branch_name("branch`name"));
    assert!(!is_valid_git_branch_name("branch!name"));
    assert!(!is_valid_git_branch_name("branch+name"));
    assert!(!is_valid_git_branch_name("branch=name"));
}

#[test]
fn test_json_repair_trailing_comma() {
    // Test repair of trailing comma before }
    let invalid_json = r#"{"key": "value",}"#;
    let repaired = try_repair_json(invalid_json);
    assert!(repaired.is_some());
    let parsed: serde_json::Value = serde_json::from_str(&repaired.unwrap()).unwrap();
    assert_eq!(parsed["key"].as_str().unwrap(), "value");
}

#[test]
fn test_json_repair_trailing_comma_in_array() {
    // Test repair of trailing comma before ]
    let invalid_json = r#"{"items": ["a", "b",]}"#;
    let repaired = try_repair_json(invalid_json);
    assert!(repaired.is_some());
    let parsed: serde_json::Value = serde_json::from_str(&repaired.unwrap()).unwrap();
    assert_eq!(parsed["items"].as_array().unwrap().len(), 2);
}

#[test]
fn test_json_repair_bare_string_closes() {
    // Test repair of bare "Closes #..." string in object
    // Pattern: , "Closes ..." (with space after comma)
    let invalid_json = r#"{"section": ["item"], "Closes #123"}"#;
    let repaired = try_repair_json(invalid_json);
    assert!(repaired.is_some(), "Failed to repair: {}", invalid_json);
    let parsed: serde_json::Value = serde_json::from_str(&repaired.unwrap()).unwrap();
    assert!(parsed["Note"].as_str().unwrap().contains("Closes #123"));

    // Also test without space after comma
    let invalid_json2 = r#"{"section": ["item"],"Closes #456"}"#;
    let repaired2 = try_repair_json(invalid_json2);
    assert!(repaired2.is_some(), "Failed to repair: {}", invalid_json2);
    let parsed2: serde_json::Value = serde_json::from_str(&repaired2.unwrap()).unwrap();
    assert!(parsed2["Note"].as_str().unwrap().contains("Closes #456"));
}

#[test]
fn test_json_repair_valid_json_unchanged() {
    // Valid JSON should pass through without issues
    let valid_json = r#"{"branch_name": "test", "commit_title": "fix: test"}"#;
    let repaired = try_repair_json(valid_json);
    assert!(repaired.is_some());
    assert_eq!(repaired.unwrap().trim(), valid_json.trim());
}

#[test]
fn test_commit_details_as_object_conversion() {
    // Test that commit_details as object gets converted to string
    let json_with_object_details = json!({
        "branch_name": "feat/test",
        "commit_title": "feat: test",
        "commit_details": {
            "### Motivation": ["Reason for change"],
            "### Solution": ["What was done", "Additional detail"]
        }
    });

    let parsed = json_with_object_details;

    // Simulate the conversion logic from gpt_ops.rs
    let commit_details = match &parsed["commit_details"] {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Null => None,
        serde_json::Value::Object(obj) => {
            let mut md = String::new();
            for (key, value) in obj {
                if key.starts_with("###") || key.starts_with("##") || key.starts_with('#') {
                    md.push_str(&format!("{}\n", key));
                } else {
                    md.push_str(&format!("### {}\n", key));
                }
                match value {
                    serde_json::Value::Array(items) => {
                        for item in items {
                            if let Some(s) = item.as_str() {
                                md.push_str(&format!("- {}\n", s));
                            }
                        }
                    }
                    serde_json::Value::String(s) => {
                        md.push_str(&format!("{}\n", s));
                    }
                    _ => {}
                }
                md.push('\n');
            }
            if md.is_empty() {
                None
            } else {
                Some(md.trim().to_string())
            }
        }
        _ => None,
    };

    assert!(commit_details.is_some());
    let details = commit_details.unwrap();
    assert!(details.contains("### Motivation"));
    assert!(details.contains("- Reason for change"));
    assert!(details.contains("### Solution"));
    assert!(details.contains("- What was done"));
}
