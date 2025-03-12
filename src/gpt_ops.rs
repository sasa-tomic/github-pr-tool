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
"You are a helpful assistant that helps to prepare GitHub Pull Requests.
You will provide output in JSON format with keys: 'branch_name', 'commit_title', and 'commit_details'.
For a very small PR return 'commit_details' as null, otherwise politely in a well structured markdown format describe all major changes for the PR.
Do not describe the impact unless there is a breaking change.
Follow the Conventional Commits specification for formatting the commit_title.
Please write in a HIGHLY CONCISE and professional style, prioritizing action-oriented verbs over longer descriptive phrases. For example:
Instead of \"introduces enhancements to functionality\" use \"extends functionality\".
Instead of \"makes modifications\" use \"updates\" .
Instead of \"provides support for\", use \"supports\".
Do not make statements that are not directly supported by the diff.
For instance, do not use \"enhances\", unless mentioned in the diff.
Do not say \"this change will improve performance\" unless the diff clearly claims that.
TRY TO IDENTIFY the MAJOR CHANGE(s) of the PR and in the description focus only on the major changes.
If there are any side changes that had to be made in order to implement the major change, do NOT mention the side changes in the PR description. So, only mention the major changes.
If there are multiple major changes, mention all of them.
If there are changes in tests cover them with a single sentence like: \"Added tests for the above\". Similar for comments: \"Updated comments\".

Ensure clarity by avoiding redundant or overly elaborate expressions. Always be concise and to the point.
Make sure that there is NO REDUNDANT information in the description.
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
