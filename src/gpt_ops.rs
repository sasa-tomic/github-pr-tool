use crate::tui::App;
use openai::{
    chat::{ChatCompletion, ChatCompletionMessage, ChatCompletionMessageRole},
    Credentials,
};

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
        "You are a helpful assistant that helps to prepare GitHub Pull Requests.
        You will provide output in JSON format with EXACTLY the following keys: 'branch_name', 'commit_title', and 'commit_details'.

        CRITICAL: The 'branch_name' field must be a valid git branch name containing only:
        - Letters (a-z, A-Z)
        - Numbers (0-9)
        - Hyphens (-)
        - Underscores (_)
        - Up to 1 forward slash (/)
        - Dots (.) but not at the beginning or end, and not consecutive

        The branch name MUST NOT contain spaces, parentheses, colons, or any other special characters.
        Examples of valid branch names: 'feat/worktree-update', 'fix-memory-leak', 'feature/add-validation', 'release/v1.0.0'
        Examples of INVALID branch names: 'feat(worktree): update', 'fix memory leak', 'feature: add validation', '.hidden', 'branch.', 'branch..name'

        For a very small PR return 'commit_details' as null, otherwise politely in a MINIMAL and well structured markdown format describe all major changes for the PR. The description should include the section 'Problem (Why?)', 'Solution (What?)', and 'Details (How?)'. Do not force the sections to be present if you cannot extrapolate MEANINGFUL and VALUABLE section information from the provided input. Ensure only HIGHLY RELEVANT information for the reviewer is included. Leave a TODO for the sections where you do not have enough information to fill them in.
        If there is a breaking change, add the 'Impact' section.

        If open GitHub issues are provided, analyze them and append a line to commit_details:
        1. If changes are related to issue #X, add 'Relates to #X'
        2. If changes completely address and close issue #X, add 'Closes #X'
        3. Only reference truly relevant issues - don't force connections.
        4. If no issues are relevant, do not append the above lines.
        5. If more than 1 issue is relevant, referenced them as 'Relates to #X, #Y' or 'Closes #X, #Y'.

        Strictly follow the Conventional Commits specification for formatting the commit_title. Commit messages should include the scope and if needed '!' to draw attention to breaking change. For instance:
        'feat(api)!: send an email to the customer when a product is shipped'
        Please write in a HIGHLY CONCISE and professional style, prioritizing action-oriented verbs over longer descriptive phrases. For example:
        Instead of \"introduces enhancements to functionality\" use \"extends functionality\".
        Instead of \"makes modifications\" use \"updates\" .
        Instead of \"provides support for\", use \"supports\".
        Do not make statements that are not directly supported by the diff.
        For instance, do not use word \"enhances\", unless mentioned in the diff.
        Do not say \"this change will improve performance\" unless the diff clearly claims that.
        TRY TO IDENTIFY the MAJOR CHANGE(s) of the PR and in the description focus only on the major changes. Do not mention the minor changes or details (such as refactoring or updating tests or documentation) unless they are the primary focus of the PR.
        If there are multiple major changes, mention all of them.
        OMIT details about the changes in comments or tests unless they are the primary focus of the PR.
        If there are changes in comments or tests mention such changes with a single line such as \"updated tests accordingly\" or \"updated comments\".

        Ensure clarity by avoiding redundant or overly elaborate expressions. Always be concise and to the point.
        Make sure that there is NO REDUNDANT or obvious information in the description. Ensure that every word in the description is necessary and adds value.
        "
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
    let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "o4-mini".to_string());

    let chat_request = ChatCompletion::builder(&model, messages.clone())
        .credentials(credentials.clone())
        .create()
        .await?;
    let chat_response = chat_request
        .choices
        .first()
        .unwrap()
        .message
        .content
        .clone()
        .unwrap_or_default();

    let chat_response = chat_response
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .to_string();

    app.add_log("INFO", format!("chat_response: {}", chat_response));
    // Parse the JSON response
    let parsed_response: serde_json::Value = match serde_json::from_str(&chat_response) {
        Ok(value) => value,
        Err(err) => {
            app.add_error(err.to_string());
            app.switch_to_tab(1);
            return Err(err.into());
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
    let commit_details = parsed_response["commit_details"]
        .as_str()
        .map(|s| s.to_string());

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
