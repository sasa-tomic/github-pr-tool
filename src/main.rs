use log::{error, info};
use openai::{
    chat::{ChatCompletion, ChatCompletionMessage, ChatCompletionMessageRole},
    Credentials,
};
use std::process::Command;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logger();
    // Ensure the OpenAI key is set
    if std::env::var("OPENAI_KEY").is_err() {
        error!("Environment variable OPENAI_KEY is not set.");
        std::process::exit(1);
    }

    // Ensure we are in a git repository
    let git_status = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()?;

    if !git_status.status.success() {
        error!("Not a git repository.");
        return Ok(());
    }

    let main_branch = git_main_branch().unwrap_or_else(|_| "main".to_string());

    if main_branch.is_empty() {
        error!("Unable to determine the upstream main branch.");
        std::process::exit(1);
    }

    let current_branch = git_current_branch()?;

    info!(
        "Main branch: {}, current branch: {}",
        main_branch, current_branch
    );

    let git_diff_string = fetch_diff_context(&main_branch, &current_branch)?;
    info!("git diff: {}", git_diff_string);

    if current_branch == main_branch && git_diff_string.is_empty() {
        info!("No changes to commit.");
        std::process::exit(0);
    }

    let (branch_name_str, commit_title, commit_details) =
        gpt_generate_branch_name_and_commit_description(git_diff_string)
            .await
            .unwrap_or((
                "my-pr-branch".to_string(),
                "Generic commit title".to_string(),
                None,
            ));
    info!(
        "branch {} commit title {} details {}",
        branch_name_str,
        commit_title,
        commit_details.clone().unwrap_or_default()
    );

    if current_branch == main_branch {
        // Create a new branch
        Command::new("git")
            .args(["checkout", "-b", &branch_name_str])
            .status()?;
    }

    stage_and_commit(&commit_title, &commit_details)?;

    // Create a GitHub PR, now that we have a branch and a commit locally
    let pr_status = Command::new("gh").args(["pr", "create"]).status()?;

    if pr_status.success() {
        println!("Pull request created successfully.");
    } else {
        eprintln!("Failed to create pull request.");
    }

    Ok(())
}

fn init_logger() {
    // default log level is INFO
    let log_level = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_level)).init();
}

fn fetch_diff_context(
    main_branch: &String,
    current_branch: &String,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut diff_context;

    // Check for staged changes
    let staged_diff = Command::new("git")
        .args(["diff", "--cached", "--", ":!*.lock"])
        .output()?;
    if !staged_diff.stdout.is_empty() {
        diff_context = String::from_utf8(staged_diff.stdout)?;
    } else {
        // Check for unstaged changes
        let unstaged_diff = Command::new("git")
            .args(["diff", "--", ":!*.lock"])
            .output()?;
        if !unstaged_diff.stdout.is_empty() {
            diff_context = String::from_utf8(unstaged_diff.stdout)?;
        } else {
            diff_context = String::from_utf8(
                Command::new("git")
                    .args([
                        "diff",
                        &format!("{}...{}", main_branch, current_branch),
                        "--",
                        ":!*.lock",
                    ])
                    .output()?
                    .stdout,
            )?
            .trim()
            .to_string();
        }
    }

    if diff_context.is_empty() {
        diff_context = "Empty context, suggest something creative".to_string();
    }

    Ok(diff_context)
}

fn git_main_branch() -> Result<String, Box<dyn std::error::Error>> {
    let mut main_branch_output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "origin/HEAD"])
        .output()?;

    if !main_branch_output.status.success() {
        Command::new("git")
            .args(["remote", "set-head", "origin", "--auto"])
            .status()?;

        main_branch_output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "origin/HEAD"])
            .output()?;
    }

    Ok(String::from_utf8(main_branch_output.stdout)?
        .trim()
        .to_string())
}

fn git_upstream_branch() -> Result<String, std::io::Error> {
    Ok(String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
            .output()?
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string())
}

fn git_current_branch() -> Result<String, std::io::Error> {
    Ok(String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()?
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string())
}

fn stage_and_commit(
    commit_title: &String,
    commit_details: &Option<String>,
) -> Result<(), std::io::Error> {
    Command::new("git").args(["add", "."]).status()?;

    let mut commit_message = commit_title.trim().to_string();
    if let Some(details) = commit_details {
        commit_message.push_str(&format!("\n\n{}", details.trim()));
    }

    Command::new("git")
        .args(["commit", "-m", &commit_message])
        .status()?;

    Ok(())
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
    info!("req {:#?}", messages);
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

    info!("chat_response: {}", chat_response);
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
