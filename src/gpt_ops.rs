use crate::config::AppConfig;
use crate::tui::App;
use serde::Deserialize;
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
                    return Err(err);
                }

                let delay_ms = INITIAL_DELAY_MS * 2u64.pow(attempt - 1);
                eprintln!(
                    "AI API call attempt {}/{} failed: {}. Retrying in {}ms...",
                    attempt, MAX_RETRIES, err, delay_ms
                );
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
    }

    unreachable!("Loop should have returned or exhausted retries")
}

// ─── Anthropic response types ─────────────────────────────────────────────────

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContentBlock>,
}

#[derive(Deserialize)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: Option<String>,
}

// ─── Provider dispatch ────────────────────────────────────────────────────────

/// Call the configured AI provider and return the raw text response.
async fn call_ai_api(
    config: &AppConfig,
    system_message: &str,
    user_message: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    match config.provider() {
        "anthropic" => call_anthropic(config, system_message, user_message).await,
        _ => call_openai(config, system_message, user_message).await,
    }
}

/// Call the Anthropic Messages API directly via HTTP.
async fn call_anthropic(
    config: &AppConfig,
    system_message: &str,
    user_message: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let api_key = config
        .ai
        .api_key
        .as_deref()
        .ok_or("Anthropic API key not set. Set `api_key` in ~/.config/gh-autopr/config.toml or ANTHROPIC_API_KEY env var.")?
        .to_string();

    let base_url = config
        .ai
        .base_url
        .as_deref()
        .unwrap_or("https://api.anthropic.com")
        .to_string();
    let url = format!("{}/v1/messages", base_url.trim_end_matches('/'));
    let model = config.model().to_string();
    let system = system_message.to_string();
    let user = user_message.to_string();

    let response_text = retry_with_backoff(|| {
        let model = model.clone();
        let system = system.clone();
        let user = user.clone();
        let api_key = api_key.clone();
        let url = url.clone();
        Box::pin(async move {
            let body = serde_json::json!({
                "model": model,
                "max_tokens": 2048,
                "system": system,
                "messages": [{"role": "user", "content": user}]
            });

            let client = reqwest::Client::new();
            let resp = client
                .post(&url)
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2023-06-01") // stable API version
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Anthropic HTTP error: {}", e))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(format!("Anthropic API error {}: {}", status, text));
            }

            let parsed: AnthropicResponse = resp
                .json()
                .await
                .map_err(|e| format!("Anthropic response parse error: {}", e))?;

            parsed
                .content
                .into_iter()
                .find(|b| b.block_type == "text")
                .and_then(|b| b.text)
                .ok_or_else(|| "Anthropic API returned no text content".to_string())
        })
    })
    .await
    .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;

    Ok(response_text)
}

// ─── OpenAI-compatible response types ────────────────────────────────────────
//
// Defined with only the fields we need; serde ignores unknown fields by
// default, so extra fields like `reasoning_content` from thinking models
// are transparently dropped.

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[derive(Deserialize)]
struct OpenAiMessage {
    content: Option<String>,
}

/// Call any OpenAI-compatible chat completions endpoint via direct HTTP.
///
/// Using `reqwest` directly means:
/// - The raw response body is always available for error messages.
/// - Unknown fields from non-standard providers (e.g. `reasoning_content`)
///   are silently ignored rather than causing a parse failure.
async fn call_openai(
    config: &AppConfig,
    system_message: &str,
    user_message: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let api_key = config
        .ai
        .api_key
        .as_deref()
        .ok_or("OpenAI API key not set. Set `api_key` in ~/.config/gh-autopr/config.toml or OPENAI_KEY env var.")?
        .to_string();

    let base_url = config
        .ai
        .base_url
        .as_deref()
        .unwrap_or("https://api.openai.com/v1")
        .to_string();
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let model = config.model().to_string();
    let system = system_message.to_string();
    let user = user_message.to_string();

    let response_text = retry_with_backoff(|| {
        let model = model.clone();
        let system = system.clone();
        let user = user.clone();
        let api_key = api_key.clone();
        let url = url.clone();
        Box::pin(async move {
            let body = serde_json::json!({
                "model": model,
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user",   "content": user},
                ]
            });

            let client = reqwest::Client::new();
            let resp = client
                .post(&url)
                .header("Authorization", format!("Bearer {}", api_key))
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("OpenAI HTTP error: {}", e))?;

            let status = resp.status();
            let raw = resp
                .text()
                .await
                .unwrap_or_else(|_| "<failed to read body>".to_string());

            if !status.is_success() {
                return Err(format!("OpenAI API error HTTP {}: {}", status, raw));
            }

            let parsed: OpenAiResponse = serde_json::from_str(&raw).map_err(|e| {
                format!("OpenAI response parse error: {}\nRaw body: {}", e, raw)
            })?;

            parsed
                .choices
                .into_iter()
                .next()
                .and_then(|c| c.message.content)
                .ok_or_else(|| {
                    format!("OpenAI API returned no choices or empty content.\nRaw body: {}", raw)
                })
        })
    })
    .await
    .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;

    Ok(response_text)
}

// ─── JSON helpers ─────────────────────────────────────────────────────────────

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
            if c == ',' {
                let rest: String = chars.clone().collect();
                let trimmed = rest.trim_start();
                if trimmed.starts_with('}') || trimmed.starts_with(']') {
                    continue;
                }
            }
            if !c.is_whitespace() {
                prev_non_ws = c;
            }
        }

        repaired.push(c);
    }

    if serde_json::from_str::<serde_json::Value>(&repaired).is_ok() {
        return Some(repaired);
    }

    let mut aggressive = repaired.clone();

    let prefixes = ["Closes", "Relates to", "See", "Fixes", "Related to"];

    for prefix in prefixes {
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

    for c in name.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '/' | '.' => continue,
            _ => return false,
        }
    }

    true
}

// ─── Public API ───────────────────────────────────────────────────────────────

pub async fn gpt_generate_branch_name_and_commit_description(
    app: &mut App<'_>,
    config: &AppConfig,
    diff_context: String,
    issues_json: Option<String>,
    what_arg: Option<String>,
    why_arg: Option<String>,
    bigger_picture_arg: Option<String>,
) -> Result<(String, String, Option<String>), Box<dyn std::error::Error>> {
    const MAX_ISSUES_LEN: usize = 16 * 1024;

    let mut system_message = String::from(
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
        system_message.push_str(&format!("\n\nUser provided 'what': {}", what));
    }
    if let Some(why) = why_arg.clone() {
        system_message.push_str(&format!("\n\nUser provided 'why': {}", why));
    }
    if let Some(bigger_picture) = bigger_picture_arg.clone() {
        system_message.push_str(&format!(
            "\n\nUser provided 'bigger picture': {}",
            bigger_picture
        ));
    }

    let user_message = format!(
        "Context:\n{}\n\nOpen GitHub Issues:\n{}",
        diff_context,
        issues_json
            .map(|j| if j.len() > MAX_ISSUES_LEN {
                j[..MAX_ISSUES_LEN].to_string()
            } else {
                j
            })
            .unwrap_or_else(|| "No open issues".to_string())
    );

    app.add_log(
        "INFO",
        format!("Calling {} ({})", config.provider(), config.model()),
    );

    let chat_response = call_ai_api(config, &system_message, &user_message)
        .await
        .inspect_err(|e| {
            app.add_error(e.to_string());
            app.switch_to_tab(1);
        })?;

    let chat_response = chat_response
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .to_string();

    app.add_log("INFO", format!("chat_response: {}", chat_response));

    let parsed_response: serde_json::Value = match serde_json::from_str(&chat_response) {
        Ok(value) => value,
        Err(err) => {
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

    let commit_details = match &parsed_response["commit_details"] {
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

    if !is_valid_git_branch_name(&branch_name) {
        let error_msg = format!(
            "AI returned invalid branch name: '{}'. Branch names must only contain letters, \
             numbers, hyphens, underscores, and forward slashes.",
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
