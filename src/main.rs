use openai::{
    chat::{ChatCompletion, ChatCompletionMessage, ChatCompletionMessageRole},
    Credentials,
};
use std::path::Path;
use std::process::Command;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Ensure the OpenAI key is set
    if std::env::var("OPENAI_KEY").is_err() {
        eprintln!("Environment variable OPENAI_KEY is not set.");
        std::process::exit(1);
    }

    // Ensure we are in a git repository
    if !Path::new(".git").exists() {
        eprintln!("Not a git repository.");
        return Ok(());
    }

    // Check for staged changes
    let staged_diff = Command::new("git").args(["diff", "--cached"]).output()?;

    if staged_diff.stdout.is_empty() {
        // No staged changes, check for unstaged changes
        let unstaged_diff = Command::new("git").args(["diff"]).output()?;

        if unstaged_diff.stdout.is_empty() {
            println!("No changes to show.");
            return Ok(());
        } else {
            // Ask user if unstaged changes should be staged
            println!("No staged changes found. Stage all unstaged changes? (y/n):");
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;

            if input.trim().eq_ignore_ascii_case("y") {
                Command::new("git").args(["add", "."]).status()?;
            } else {
                println!("No changes staged. Exiting.");
                return Ok(());
            }
        }
    }

    // Remember the current branch
    let current_branch = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()?;

    let upstream_branch = String::from_utf8(current_branch.stdout)?.trim().to_string();

    // Generate a branch name and commit description using OpenAI
    let diff_context_str = fetch_diff_context()?;

    let (branch_name_str, commit_title, commit_details) =
        gpt_generate_branch_name_and_commit_description(diff_context_str)
            .await
            .unwrap_or((
                "my-pr-branch".to_string(),
                "Generic commit title".to_string(),
                None,
            ));

    // Create a new branch
    Command::new("git")
        .args(["checkout", "-b", &branch_name_str])
        .status()?;

    // Commit the staged changes
    let commit_message = if let Some(details) = commit_details {
        format!(
            "{}

{}",
            commit_title, details
        )
    } else {
        commit_title
    };

    Command::new("git")
        .args(["commit", "-m", &commit_message])
        .status()?;

    // Create a GitHub PR
    let pr_status = Command::new("gh")
        .args([
            "pr",
            "create",
            "--base",
            &upstream_branch,
            "--head",
            &branch_name_str,
        ])
        .status()?;

    if pr_status.success() {
        println!("Pull request created successfully.");
    } else {
        eprintln!("Failed to create pull request.");
    }

    Ok(())
}

fn fetch_diff_context() -> Result<String, Box<dyn std::error::Error>> {
    let mut diff_context = String::new();

    // Check for staged changes
    let staged_diff = Command::new("git").args(["diff", "--cached"]).output()?;
    if !staged_diff.stdout.is_empty() {
        diff_context = String::from_utf8(staged_diff.stdout)?;
    } else {
        // Check for unstaged changes
        let unstaged_diff = Command::new("git").args(["diff"]).output()?;
        if !unstaged_diff.stdout.is_empty() {
            diff_context = String::from_utf8(unstaged_diff.stdout)?;
        } else {
            // Check against upstream branch
            let upstream_branch = Command::new("git")
                .args(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
                .output();

            if let Ok(upstream_output) = upstream_branch {
                if !upstream_output.stdout.is_empty() {
                    let upstream_branch_name = String::from_utf8(upstream_output.stdout)?
                        .trim()
                        .to_string();
                    let upstream_diff = Command::new("git")
                        .args(["diff", &upstream_branch_name])
                        .output()?;
                    if !upstream_diff.stdout.is_empty() {
                        diff_context = String::from_utf8(upstream_diff.stdout)?;
                    }
                }
            }
        }
    }

    if diff_context.is_empty() {
        diff_context = "None, suggest something creative".to_string();
    }

    Ok(diff_context)
}

async fn gpt_generate_branch_name_and_commit_description(
    diff_context: String,
) -> Result<(String, String, Option<String>), Box<dyn std::error::Error>> {
    let credentials = Credentials::from_env();
    let messages = vec![
        ChatCompletionMessage {
            role: ChatCompletionMessageRole::System,
            content: Some(
                "You are a helpful assistant that helps to prepare GitHub PRs. You will provide output in JSON format with keys: 'branch_name', 'commit_title', and 'commit_details'. If the context has only one line, return 'commit_details' as null. Follow the Conventional Commits specification for formatting PR descriptions.".to_string(),
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
    println!("req {:#?}", messages);
    let chat_request = ChatCompletion::builder("gpt-4o-mini", messages.clone())
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

    println!("chat_response: {}", chat_response);
    // Parse the JSON response
    let parsed_response: serde_json::Value = serde_json::from_str(&chat_response)?;
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
