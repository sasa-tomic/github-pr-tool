# GitHub PR Tool

## Overview

The GitHub PR Tool is a Rust-based command-line utility designed to streamline the process of creating GitHub pull requests (PRs). It leverages OpenAI's language model to generate meaningful branch names and commit messages, while automating tasks such as staging changes, creating new branches, committing changes, and opening PRs via the GitHub CLI (`gh`).

## Features

- **Smart Branch Naming**: Automatically generate descriptive branch names based on the content of your changes.
- **Commit Message Assistance**: Create concise commit titles and optional detailed descriptions using OpenAI's LLM.
- **Automated Workflow**: Stage changes, create a branch, commit changes, and open a pull request with minimal effort.
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
   - Create a new branch, commit changes, and open a pull request.

## Environment Variables

- `OPENAI_KEY`: Your OpenAI API key, required for generating branch names and commit messages.

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
