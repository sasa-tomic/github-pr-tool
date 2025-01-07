use crate::tui::App;
use openai::{
    chat::{ChatCompletion, ChatCompletionMessage, ChatCompletionMessageRole},
    Credentials,
};

pub async fn gpt_generate_branch_name_and_commit_description(
    app: &mut App<'_>,
    diff_context: String,
) -> Result<(String, String, Option<String>), Box<dyn std::error::Error>> {
    let credentials = Credentials::from_env();
    let messages = vec![
        ChatCompletionMessage {
            role: ChatCompletionMessageRole::System,
            content: Some(
                "You are a helpful assistant that helps to prepare GitHub PRs.
                You will provide output in JSON format with keys: 'branch_name', 'commit_title', and 'commit_details'.
                For a very small PR return 'commit_details' as null, otherwise humbly and politely in a well structured markdown format describe all changes in the PR.
                Do not describe the impact unless there is a breaking change.
                Follow the Conventional Commits specification for formatting PR descriptions.
                Please write in a HIGHLY CONCISE and professional style, prioritizing action-oriented verbs over longer descriptive phrases. For example:
                Use \"extends functionality\" instead of \"introduces enhancements to functionality\".
                Use \"updates\" instead of \"makes modifications\".
                Use \"supports\" instead of \"provides support for\".
                Do not use *enhanced* or similar words in the descriptions, unless such statements are explicitly provided in the diff.
                Do not make statements that are not directly supported by the diff. For example, do not say \"this change will improve performance\" unless the diff clearly shows or claims that.
                Do not provide details on the comment or test changes unless they are significant, just provide a very concise high-level overview for such changes such as \"updated tests\" or \"updated comments\".
                Ensure clarity by avoiding redundant or overly elaborate expressions. Be concise and to the point.
                ".to_string(),
            ),
            ..Default::default()
        },
        ChatCompletionMessage {
            role: ChatCompletionMessageRole::User,
            content: Some(format!(
                "Context:\n{}",
                diff_context
            )),
            ..Default::default()
        },
    ];
    let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o".to_string());

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

    Ok((branch_name, commit_title, commit_details))
}
