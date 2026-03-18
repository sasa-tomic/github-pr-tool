# GitHub PR Tool

## Overview

The GitHub PR Tool is a Rust-based command-line utility designed to streamline the process of creating GitHub pull requests (PRs). It leverages OpenAI's language model to generate meaningful branch names and commit messages, while automating tasks such as staging changes, creating new branches, committing changes, and opening PRs via the GitHub CLI (`gh`).

## Features

- **Smart Branch Naming**: Automatically generate descriptive branch names based on the content of your changes.
- **Commit Message Assistance**: Create concise commit titles and optional detailed descriptions using OpenAI's LLM.
- **Automated Workflow**: Stage changes, create a branch, commit changes, run optional external review/prep, and open a pull request with minimal effort.
- **External Diff Review Gate**: Optionally run ACP/opencode/other CLI agents as a diff reviewer before push + PR creation, with support for blocking, user-feedback, autonomous prep loops, and ready-to-submit decisions.
- **Interactive Staging**: Offers to stage unstaged changes interactively if no changes are staged.
- **Error Handling**: Validates prerequisites like being inside a Git repository and having the OpenAI API key set.

## Prerequisites

- **Git**: Installed and available in your PATH.
- **GitHub CLI (`gh`)**: Installed and authenticated.
- **Rust**: Installed for building and running the tool.
- **OpenAI API Key**: Set as an environment variable (`OPENAI_KEY`).

## Installation

1. Clone the repository:

   ```bash
   git clone https://github.com/sasa-tomic/github-pr-tool.git
   cd github-pr-tool
   ```

2. Build the project:

   ```bash
   cargo build --release
   ```

3. Run the tool:

   ```bash
   ./target/release/github-pr-tool
   ```

## Usage

1. Ensure you are inside a Git repository.
2. Run the tool:

   ```bash
   ./target/release/github-pr-tool
   ```

3. The tool will:
   - Check for staged changes.
   - If none are staged, interactively ask to stage unstaged changes.
   - Generate a branch name and commit message based on the changes.
   - Optionally run an external reviewer (`--review-command`) on the branch diff and enforce one of: block submission, request user feedback, autonomously prepare/amend, or proceed.
   - Create/update a pull request only when the review verdict is ready for submission.

### Optional Review Command

Configure `[review]` in `~/.config/gh-autopr/config.toml` to run review automatically every time. Review is enabled by default; set `enabled = false` to disable it. Use `--review-command` to override per-run when review is enabled.

You can invoke any CLI reviewer (for example ACP, `opencode`, or `ralph`) that reads a prompt from stdin and returns strict JSON with a decision.

```bash
gh-autopr --review-command "opencode run --json" --review-max-rounds 3
```


### User-level config example

```toml
[review]
enabled = true
command = "opencode run --json"
max_rounds = 3
```

Supported decisions:
- `not_worth_submission`: stop and report feedback, no PR submitted
- `needs_user_feedback`: stop and present reviewer questions, no PR submitted
- `needs_autonomous_prep`: run provided prep commands, amend commit, then re-review (loop)
- `ready_for_submission`: continue to push + PR creation

## Environment Variables

- `OPENAI_KEY`: Your OpenAI API key, required for generating branch names and commit messages.
- `AUTOPR_REVIEW_ENABLED`: Optional review toggle (`true/false`); defaults to enabled.

## Example

```bash
> ./target/release/github-pr-tool
No staged changes found. Stage all unstaged changes? (y/n): y
Pull request created successfully.
```

## Contributing

Contributions are welcome! Here’s how you can help:

1. Fork the repository.
2. Create a feature branch:

   ```bash
   git checkout -b feature/your-feature
   ```

3. Make your changes and test thoroughly.
4. Submit a pull request.

## License

This project is licensed under the MIT License. See the `LICENSE` file for details.

## Acknowledgments

- Built with Rust and powered by OpenAI's GPT technology.
