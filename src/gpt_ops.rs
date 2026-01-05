use crate::tui::App;
use openai::{
    chat::{ChatCompletion, ChatCompletionMessage, ChatCompletionMessageRole},
    Credentials,
};
use std::time::Duration;

/// Retries an async operation up to MAX_RETRIES times with exponential backoff.
/// Initial delay: 1s, then 2s, then 4s (for 3 total attempts).
async fn retry_with_backoff<F, T, E>(mut operation: F) -> Result<T, E>
where
    F: FnMut() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, E>> + Send>>,
    E: std::fmt::Display,
{
    const MAX_RETRIES: u32 = 3;
    const INITIAL_DELAY_MS: u64 = 1000;

    for attempt in 1..=MAX_RETRIES {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(err) => {
                if attempt == MAX_RETRIES {
                    // Exhausted all retries, fail
                    return Err(err);
                }

                // Calculate exponential backoff: 1s, 2s, 4s
                let delay_ms = INITIAL_DELAY_MS * 2u64.pow(attempt - 1);
                eprintln!(
                    "GPT API call attempt {}/{} failed: {}. Retrying in {}ms...",
                    attempt, MAX_RETRIES, err, delay_ms
                );
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
    }

    unreachable!("Loop should have returned or exhausted retries")
}

/// Attempts to repair common GPT JSON mistakes
///
/// Common issues:
/// - Trailing commas before closing braces/brackets
/// - Bare strings in objects (missing key)
fn try_repair_json(json: &str) -> Option<String> {
    let mut repaired = String::with_capacity(json.len() + 50);
    let mut chars = json.chars().peekable();
    let mut in_string = false;
    let mut prev_non_ws = ' ';

    while let Some(c) = chars.next() {
        if c == '"' && prev_non_ws != '\\' {
            in_string = !in_string;
        }

        if !in_string {
            // Remove trailing commas before } or ]
            if c == ',' {
                // Look ahead to see if next non-whitespace is } or ]
                let rest: String = chars.clone().collect();
                let trimmed = rest.trim_start();
                if trimmed.starts_with('}') || trimmed.starts_with(']') {
                    // Skip this comma
                    continue;
                }
            }
            if !c.is_whitespace() {
                prev_non_ws = c;
            }
        }

        repaired.push(c);
    }

    // Try parsing the repaired JSON
    if serde_json::from_str::<serde_json::Value>(&repaired).is_ok() {
        return Some(repaired);
    }

    // More aggressive repair: try to fix bare strings in objects
    // Pattern: look for ,"string"} and convert to ,"Note":"string"}
    let mut aggressive = repaired.clone();

    // Find patterns like ,"..." followed eventually by } without a : in between
    // This is a heuristic that handles the specific GPT error we've seen
    // Handle both with and without whitespace: `,"Closes` or `, "Closes`
    let prefixes = ["Closes", "Relates to", "See", "Fixes", "Related to"];

    for prefix in prefixes {
        // Pattern: , "Prefix ..." } (with possible whitespace)
        let patterns = [
            (
                format!(r#", "{}"#, prefix),
                format!(r#", "Note": "{}"#, prefix),
            ),
            (
                format!(r#","{}"#, prefix),
                format!(r#","Note": "{}"#, prefix),
            ),
        ];
        for (from, to) in patterns {
            aggressive = aggressive.replace(&from, &to);
        }
    }

    if serde_json::from_str::<serde_json::Value>(&aggressive).is_ok() {
        return Some(aggressive);
    }

    None
}

/// Validates if a string is a valid git branch name
///
/// Git branch names must not contain:
/// - Spaces
/// - Special characters like :, (, ), [, ], {, }, ?, *, ^, ~, \, etc.
/// - Start or end with dots
/// - Consecutive dots
/// - Be empty or just a dash
fn is_valid_git_branch_name(name: &str) -> bool {
    if name.is_empty() || name == "-" {
        return false;
    }

    if name.starts_with('.') || name.ends_with('.') {
        return false;
    }

    if name.contains("..") {
        return false;
    }

    // Check for invalid characters
    for c in name.chars() {
        match c {
            // Allow alphanumeric, hyphens, underscores, forward slashes, and dots
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '/' | '.' => continue,
            _ => return false,
        }
    }

    true
}

pub async fn gpt_generate_branch_name_and_commit_description(
    app: &mut App<'_>,
    diff_context: String,
    issues_json: Option<String>,
    what_arg: Option<String>,
    why_arg: Option<String>,
    bigger_picture_arg: Option<String>,
) -> Result<(String, String, Option<String>), Box<dyn std::error::Error>> {
    const MAX_ISSUES_LEN: usize = 16 * 1024; // 16K characters limit for issues
    let credentials = Credentials::from_env();
    let mut system_message_content = String::from(
        r#"You prepare concise GitHub Pull Requests.

OUTPUT
Return valid JSON with EXACTLY these keys: "branch_name", "commit_title", "commit_details".
IMPORTANT: All values must be strings (or null for commit_details). Do NOT use nested objects or arrays.

BRANCH NAME (strict)
- Allowed: letters, digits, hyphen (-), underscore (_), dot (.), up to ONE forward slash (/).
- Not allowed: spaces, parentheses, colons, other symbols.
- No leading/trailing dot. No consecutive dots. Max one "/".
Examples OK: feat/worktree-update, fix-memory-leak, release/v1.0.0
Examples NO: feat(worktree): update, fix memory leak, .hidden, branch., branch..name

COMMIT TITLE (Conventional Commits)
- Format: <type>(<scope>)!?: <imperative summary>
- Use "!" if breaking.
- ≤ 72 chars. Action verbs. No fluff. Only claims supported by the diff.

COMMIT DETAILS (a single Markdown string, NOT a nested object)
- Value must be a single string containing Markdown, or null. Never an object or array.
- If the PR is truly tiny AND no issue refs: set "commit_details" to null.
- Otherwise write ≤ 120 words total AND ≤ 8 lines. Prefer bullets. No code blocks.
- Include ONLY sections that add high value; Exclude those with low and medium value. Section order:
  - ### Motivation (≤ 1 bullet in the section)
  - ### Solution (1-3 bullets)
  - ### Impact (include ONLY if breaking; note migration in ≤ 1 bullet)
  - ### Details (0-3 non-obvious bullets; skip routine refactors)
  - ### Meta — single line if needed: "updated tests accordingly" or "updated comments".
- Do NOT restate obvious diffs. Do NOT claim perf/security/UX benefits unless explicit in the diff.
- Focus on MAJOR change(s). Minor changes only if they are the main point.

GITHUB ISSUES (append at end)
- If relevant: add exactly one line: "Relates to #X[, #Y]" or "Closes #X[, #Y]".
- Only when truly connected. If multiple, comma-separate numbers.
- If the PR is tiny BUT has relevant issues, include ONLY this line (do not use null).

STYLE
- Crisp, professional, fun-but-sparing. No filler ("this PR", "in order to", etc.).
"#,
    );

    if let Some(what) = what_arg.clone() {
        system_message_content.push_str(&format!("\n\nUser provided 'what': {}", what));
    }
    if let Some(why) = why_arg.clone() {
        system_message_content.push_str(&format!("\n\nUser provided 'why': {}", why));
    }
    if let Some(bigger_picture) = bigger_picture_arg.clone() {
        system_message_content.push_str(&format!(
            "\n\nUser provided 'bigger picture': {}",
            bigger_picture
        ));
    }

    let messages = vec![
        ChatCompletionMessage {
            role: ChatCompletionMessageRole::System,
            content: Some(system_message_content),
            ..Default::default()
        },
        ChatCompletionMessage {
            role: ChatCompletionMessageRole::User,
            content: Some(format!(
                "Context:\n{}\n\nOpen GitHub Issues:\n{}",
                diff_context,
                issues_json
                    .map(|j| if j.len() > MAX_ISSUES_LEN {
                        j[..MAX_ISSUES_LEN].to_string()
                    } else {
                        j
                    })
                    .unwrap_or_else(|| "No open issues".to_string())
            )),
            ..Default::default()
        },
    ];
    let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-5-mini".to_string());

    // Retry the API call with exponential backoff
    let chat_request = retry_with_backoff(|| {
        let model = model.clone();
        let messages = messages.clone();
        let credentials = credentials.clone();
        Box::pin(async move {
            ChatCompletion::builder(&model, messages)
                .credentials(credentials)
                .create()
                .await
        })
    })
    .await?;

    let first_choice = chat_request.choices.first().ok_or_else(|| {
        // Get base URL and API key for debugging
        let base_url = std::env::var("OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
        let api_key = std::env::var("OPENAI_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .unwrap_or_else(|_| "not set".to_string());

        // Show first 8 characters of the token for debugging (safe to show)
        let token_preview = if api_key != "not set" && api_key.len() >= 8 {
            format!("{}...", &api_key[..8])
        } else {
            api_key.clone()
        };

        let error_msg = format!(
            "OpenAI API returned no choices. This may indicate:\n\
                - Invalid model name: '{}'\n\
                - Invalid base URL configuration\n\
                - API authentication error\n\
                - API rate limit or service error\n\
                \n\
                Debug info:\n\
                - Base URL: {}\n\
                - API Key: {}\n\
                \n\
                Check your OPENAI_MODEL, OPENAI_BASE_URL, and OPENAI_KEY environment variables.",
            model, base_url, token_preview
        );
        app.add_error(error_msg.clone());
        app.switch_to_tab(1);
        std::io::Error::other(error_msg)
    })?;

    let chat_response = first_choice.message.content.clone().unwrap_or_default();

    let chat_response = chat_response
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .to_string();

    app.add_log("INFO", format!("chat_response: {}", chat_response));
    // Parse the JSON response, attempting repair if needed
    let parsed_response: serde_json::Value = match serde_json::from_str(&chat_response) {
        Ok(value) => value,
        Err(err) => {
            // Try to repair common GPT JSON mistakes
            app.add_log(
                "WARN",
                format!("JSON parse failed: {}, attempting repair", err),
            );
            match try_repair_json(&chat_response) {
                Some(repaired) => {
                    app.add_log("INFO", "JSON repair succeeded");
                    match serde_json::from_str(&repaired) {
                        Ok(value) => value,
                        Err(err2) => {
                            app.add_error(format!(
                                "JSON repair failed: {}\nResponse was:\n{}",
                                err2, chat_response
                            ));
                            app.switch_to_tab(1);
                            return Err(err2.into());
                        }
                    }
                }
                None => {
                    app.add_error(format!(
                        "JSON parse error: {}\nResponse was:\n{}",
                        err, chat_response
                    ));
                    app.switch_to_tab(1);
                    return Err(err.into());
                }
            }
        }
    };
    let branch_name = parsed_response["branch_name"]
        .as_str()
        .unwrap_or("my-pr-branch")
        .to_string();
    let commit_title = parsed_response["commit_title"]
        .as_str()
        .unwrap_or("Generic commit title")
        .to_string();
    // Handle commit_details - can be string, null, or (incorrectly) an object
    let commit_details = match &parsed_response["commit_details"] {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Null => None,
        serde_json::Value::Object(obj) => {
            // GPT sometimes returns structured data; convert to Markdown string
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

    // Validate the branch name
    if !is_valid_git_branch_name(&branch_name) {
        let error_msg = format!(
            "GPT returned invalid branch name: '{}'. Branch names must only contain letters, numbers, hyphens, underscores, and forward slashes. No spaces, parentheses, colons, or other special characters allowed.",
            branch_name
        );
        app.add_error(error_msg.clone());
        app.switch_to_tab(1);
        return Err(error_msg.into());
    }

    Ok((branch_name, commit_title, commit_details))
}

#[cfg(test)]
#[path = "gpt_ops/tests.rs"]
mod tests;
