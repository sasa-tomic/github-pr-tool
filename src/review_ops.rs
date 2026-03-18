use crate::tui::App;
use serde::Deserialize;
use std::process::{Command, Stdio};

#[derive(Debug, Clone)]
pub struct ExternalReviewConfig {
    pub review_command: Option<String>,
    pub max_rounds: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewDecision {
    NotWorthSubmission,
    NeedsUserFeedback,
    NeedsAutonomousPrep,
    ReadyForSubmission,
}

#[derive(Debug, Clone)]
pub struct ReviewResult {
    pub decision: ReviewDecision,
    pub summary: String,
    pub feedback: Vec<String>,
    pub questions: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ReviewResponse {
    decision: String,
    summary: Option<String>,
    feedback: Option<Vec<String>>,
    questions: Option<Vec<String>>,
    prep_commands: Option<Vec<String>>,
}

pub fn review_and_prepare_change(
    app: &mut App<'_>,
    config: &ExternalReviewConfig,
    base_branch: &str,
    current_branch: &str,
    initial_diff: String,
) -> Result<ReviewResult, Box<dyn std::error::Error>> {
    let Some(review_command) = config.review_command.as_deref() else {
        app.add_log(
            "INFO",
            "No review command configured; skipping external diff review.",
        );
        return Ok(ReviewResult {
            decision: ReviewDecision::ReadyForSubmission,
            summary: "Review skipped (no command configured).".to_string(),
            feedback: vec![],
            questions: vec![],
        });
    };

    let mut diff = initial_diff;

    for round in 1..=config.max_rounds.max(1) {
        app.add_log(
            "INFO",
            format!("External review round {}/{}", round, config.max_rounds),
        );

        let response = run_reviewer(review_command, base_branch, current_branch, &diff)?;
        let summary = response
            .summary
            .unwrap_or_else(|| "No summary provided.".to_string());
        let feedback = response.feedback.unwrap_or_default();
        let questions = response.questions.unwrap_or_default();

        let decision = parse_decision(&response.decision)?;
        let prep_commands = response.prep_commands.unwrap_or_default();

        match decision {
            ReviewDecision::NotWorthSubmission => {
                return Ok(ReviewResult {
                    decision,
                    summary,
                    feedback,
                    questions,
                });
            }
            ReviewDecision::NeedsUserFeedback => {
                return Ok(ReviewResult {
                    decision,
                    summary,
                    feedback,
                    questions,
                });
            }
            ReviewDecision::ReadyForSubmission => {
                return Ok(ReviewResult {
                    decision,
                    summary,
                    feedback,
                    questions,
                });
            }
            ReviewDecision::NeedsAutonomousPrep => {
                if !questions.is_empty() {
                    return Ok(ReviewResult {
                        decision: ReviewDecision::NeedsUserFeedback,
                        summary,
                        feedback,
                        questions,
                    });
                }

                if prep_commands.is_empty() {
                    return Err(
                        "Reviewer requested autonomous prep but returned no prep_commands"
                            .to_string()
                            .into(),
                    );
                }

                for prep in prep_commands {
                    app.add_log("INFO", format!("Running prep command: {}", prep));
                    run_shell_command(&prep)?;
                }

                let status = Command::new("git")
                    .args(["status", "--porcelain"])
                    .output()?;
                if !status.status.success() {
                    return Err("Failed to inspect git status after prep".to_string().into());
                }

                if !String::from_utf8_lossy(&status.stdout).trim().is_empty() {
                    run_shell_command("git add -A")?;
                    run_shell_command("git commit --amend --no-edit")?;
                    app.add_log("INFO", "Amended commit with autonomous prep updates.");
                } else {
                    app.add_log("INFO", "Autonomous prep made no file changes.");
                }

                let output = Command::new("git")
                    .args(["diff", "--", &format!("{base_branch}...{current_branch}")])
                    .output()?;
                if !output.status.success() {
                    return Err("Failed to re-read branch diff after prep"
                        .to_string()
                        .into());
                }
                diff = String::from_utf8_lossy(&output.stdout).to_string();
            }
        }
    }

    Err(
        "Review did not converge to ready/blocking state within max review rounds"
            .to_string()
            .into(),
    )
}

fn parse_decision(raw: &str) -> Result<ReviewDecision, Box<dyn std::error::Error>> {
    match raw.trim() {
        "not_worth_submission" => Ok(ReviewDecision::NotWorthSubmission),
        "needs_user_feedback" => Ok(ReviewDecision::NeedsUserFeedback),
        "needs_autonomous_prep" => Ok(ReviewDecision::NeedsAutonomousPrep),
        "ready_for_submission" => Ok(ReviewDecision::ReadyForSubmission),
        other => Err(format!("Unknown review decision: {}", other).into()),
    }
}

fn run_reviewer(
    command: &str,
    base_branch: &str,
    current_branch: &str,
    diff: &str,
) -> Result<ReviewResponse, Box<dyn std::error::Error>> {
    let prompt = format!(
        "You are reviewing a git diff before PR submission.\\nBase branch: {}\\nCurrent branch: {}\\n\\nReturn ONLY strict JSON with this schema:\\n{{\\n  \"decision\": \"not_worth_submission\" | \"needs_user_feedback\" | \"needs_autonomous_prep\" | \"ready_for_submission\",\\n  \"summary\": string,\\n  \"feedback\": string[],\\n  \"questions\": string[],\\n  \"prep_commands\": string[]\\n}}\\n\\nRules:\\n- Use not_worth_submission if this change should not be submitted as a PR.\\n- Use needs_user_feedback if submission could be worth it but user clarification is required.\\n- Use needs_autonomous_prep when no user input is needed but local prep should happen before PR submission.\\n- Use ready_for_submission if PR can be created now.\\n- If decision is needs_user_feedback, put user-facing questions in questions.\\n- If decision is needs_autonomous_prep, include shell prep_commands to run from repo root and keep questions empty.\\n\\nDiff:\\n{}",
        base_branch, current_branch, diff
    );

    let mut child = Command::new("bash")
        .arg("-lc")
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    use std::io::Write;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(prompt.as_bytes())?;
    }

    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Review command failed: {}", stderr).into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: ReviewResponse = serde_json::from_str(stdout.trim()).map_err(|e| {
        format!(
            "Review command did not return valid JSON: {}. Output: {}",
            e, stdout
        )
    })?;

    Ok(parsed)
}

fn run_shell_command(cmd: &str) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("bash").arg("-lc").arg(cmd).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Command failed `{}`: {}", cmd, stderr).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_decisions() {
        assert_eq!(
            parse_decision("ready_for_submission").unwrap(),
            ReviewDecision::ReadyForSubmission
        );
        assert!(parse_decision("bogus").is_err());
    }
}
