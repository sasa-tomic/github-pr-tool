use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Top-level application configuration, loaded from `~/.config/gh-autopr/config.toml`
/// and overridden by environment variables.
///
/// Example config file:
/// ```toml
/// [ai]
/// provider = "anthropic"       # "openai" (default) or "anthropic"
/// api_key  = "sk-ant-..."      # API key (prefer env var or keyring over plaintext)
/// model    = "claude-opus-4-6" # model name; see https://docs.anthropic.com/en/docs/about-claude/models
/// base_url = "https://..."     # optional custom endpoint
///
/// [review]
/// enabled = true                    # optional: default true; set false to skip review entirely
/// command = "opencode run --json" # optional: if set and enabled, review runs automatically
/// max_rounds = 2                    # optional: autonomous prep loop cap
/// ```
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub ai: AiConfig,
    #[serde(default)]
    pub review: ReviewConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct AiConfig {
    /// AI provider: "openai" (default) or "anthropic"
    pub provider: Option<String>,
    /// API key for the chosen provider
    pub api_key: Option<String>,
    /// Model name (e.g. "gpt-4o-mini" or "claude-opus-4-6")
    pub model: Option<String>,
    /// Optional custom base URL (e.g. for local proxies or compatible endpoints)
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReviewConfig {
    /// Whether review is enabled. Defaults to true.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// External review command (reads prompt from stdin, outputs strict JSON)
    pub command: Option<String>,
    /// Max autonomous prep rounds before giving up
    pub max_rounds: Option<u32>,
}

fn default_true() -> bool {
    true
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            command: None,
            max_rounds: None,
        }
    }
}

const STUB: &str = r#"# gh-autopr configuration — edit this file, then re-run gh-autopr.

[ai]
# AI provider: "anthropic" or "openai"
provider = "anthropic"

# API key. You can also set ANTHROPIC_API_KEY in your environment instead.
api_key = ""

# Model name. Verify current names at:
#   https://docs.anthropic.com/en/docs/about-claude/models
model = "claude-opus-4-6"

# Optional: override the API base URL (useful for proxies or compatible endpoints).
# base_url = "https://api.anthropic.com"

# ── OpenAI (uncomment all lines below and remove the anthropic settings above) ──
# provider = "openai"
# api_key  = ""        # or set OPENAI_KEY in your environment
# model    = "gpt-4o-mini"
# base_url = "https://api.openai.com/v1"  # optional

[review]
# Review is enabled by default. Set to false to skip review entirely.
enabled = true

# Optional: if set, diff review runs automatically before PR creation.
# Works with ACP/opencode/ralph CLI/any agent that reads stdin and outputs strict JSON.
# command = "opencode run --json"

# Optional autonomous prep loop cap.
# max_rounds = 2

# If you prefer the ralph CLI, put your command wrapper here, e.g.:
# command = "ralph run --json"
"#;

impl AppConfig {
    /// If the config file does not exist, write a stub and return `true`.
    /// Returns `false` if the file already existed.
    /// The caller should print a message and exit when this returns `true`.
    pub fn ensure_stub() -> Result<bool, Box<dyn std::error::Error>> {
        let path = match Self::config_file_path() {
            Some(p) => p,
            None => return Ok(false),
        };

        if path.exists() {
            return Ok(false);
        }

        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, STUB)?;
        Ok(true)
    }

    /// Load config from `~/.config/gh-autopr/config.toml`, then apply env var overrides.
    /// Missing or unreadable config files are silently ignored; parse errors are printed to stderr.
    pub fn load() -> Self {
        let mut config = Self::default();

        if let Some(path) = Self::config_file_path() {
            if path.exists() {
                match std::fs::read_to_string(&path) {
                    Ok(content) => match toml::from_str::<AppConfig>(&content) {
                        Ok(file_config) => config = file_config,
                        Err(e) => eprintln!(
                            "Warning: failed to parse config file {}: {}",
                            path.display(),
                            e
                        ),
                    },
                    Err(e) => eprintln!(
                        "Warning: failed to read config file {}: {}",
                        path.display(),
                        e
                    ),
                }
            }
        }

        config.apply_env_overrides();
        config
    }

    /// Path to the config file: `~/.config/gh-autopr/config.toml`
    pub fn config_file_path() -> Option<PathBuf> {
        dirs::config_dir().map(|p| p.join("gh-autopr").join("config.toml"))
    }

    /// Apply environment variable overrides (highest priority over config file).
    ///
    /// Variable priority:
    /// - Provider:  `AUTOPR_PROVIDER`
    /// - API key:   `AUTOPR_API_KEY` > provider-specific fallback
    /// - Model:     `AUTOPR_MODEL` > provider-specific fallback
    /// - Base URL:  `AUTOPR_BASE_URL` > provider-specific fallback
    /// - Review enabled: `AUTOPR_REVIEW_ENABLED`
    /// - Review cmd: `AUTOPR_REVIEW_COMMAND`
    /// - Review rounds: `AUTOPR_REVIEW_MAX_ROUNDS`
    ///
    /// Provider-specific fallbacks:
    /// - anthropic: `ANTHROPIC_API_KEY`, `ANTHROPIC_MODEL`, `ANTHROPIC_BASE_URL`
    /// - openai:    `OPENAI_KEY`/`OPENAI_API_KEY`, `OPENAI_MODEL`, `OPENAI_BASE_URL`
    fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("AUTOPR_PROVIDER") {
            self.ai.provider = Some(v);
        }

        if let Ok(v) = std::env::var("AUTOPR_API_KEY") {
            self.ai.api_key = Some(v);
        } else if self.ai.api_key.is_none() {
            let key = match self.provider() {
                "anthropic" => std::env::var("ANTHROPIC_API_KEY").ok(),
                _ => std::env::var("OPENAI_KEY")
                    .or_else(|_| std::env::var("OPENAI_API_KEY"))
                    .ok(),
            };
            if let Some(k) = key {
                self.ai.api_key = Some(k);
            }
        }

        if let Ok(v) = std::env::var("AUTOPR_MODEL") {
            self.ai.model = Some(v);
        } else if self.ai.model.is_none() {
            let model = match self.provider() {
                "anthropic" => std::env::var("ANTHROPIC_MODEL").ok(),
                _ => std::env::var("OPENAI_MODEL").ok(),
            };
            if let Some(m) = model {
                self.ai.model = Some(m);
            }
        }

        if let Ok(v) = std::env::var("AUTOPR_BASE_URL") {
            self.ai.base_url = Some(v);
        } else if self.ai.base_url.is_none() {
            let url = match self.provider() {
                "anthropic" => std::env::var("ANTHROPIC_BASE_URL").ok(),
                _ => std::env::var("OPENAI_BASE_URL").ok(),
            };
            if let Some(u) = url {
                self.ai.base_url = Some(u);
            }
        }

        if let Ok(v) = std::env::var("AUTOPR_REVIEW_ENABLED") {
            let normalized = v.trim().to_ascii_lowercase();
            self.review.enabled = matches!(normalized.as_str(), "1" | "true" | "yes" | "on");
        }

        if let Ok(v) = std::env::var("AUTOPR_REVIEW_COMMAND") {
            self.review.command = Some(v);
        }

        if let Ok(v) = std::env::var("AUTOPR_REVIEW_MAX_ROUNDS") {
            if let Ok(parsed) = v.parse::<u32>() {
                self.review.max_rounds = Some(parsed.max(1));
            }
        }
    }

    /// Effective provider (defaults to "openai").
    pub fn provider(&self) -> &str {
        self.ai.provider.as_deref().unwrap_or("openai")
    }

    /// Effective model, using a sensible default for the provider.
    pub fn model(&self) -> &str {
        self.ai
            .model
            .as_deref()
            .unwrap_or_else(|| match self.provider() {
                // Use the model name exactly as listed in Anthropic's API docs.
                // Verify current names at https://docs.anthropic.com/en/docs/about-claude/models
                "anthropic" => "claude-opus-4-6",
                _ => "gpt-4o-mini",
            })
    }

    pub fn review_enabled(&self) -> bool {
        self.review.enabled
    }

    pub fn review_command(&self) -> Option<&str> {
        self.review.command.as_deref().and_then(|v| {
            if self.review_enabled() && !v.trim().is_empty() {
                Some(v)
            } else {
                None
            }
        })
    }

    pub fn review_max_rounds(&self) -> u32 {
        self.review.max_rounds.unwrap_or(2).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_defaults_are_stable() {
        let cfg = AppConfig::default();
        assert!(cfg.review_enabled());
        assert!(cfg.review_command().is_none());
        assert_eq!(cfg.review_max_rounds(), 2);
    }

    #[test]
    fn review_command_trims_empty_values() {
        let mut cfg = AppConfig::default();
        cfg.review.command = Some("   ".to_string());
        assert!(cfg.review_command().is_none());
    }

    #[test]
    fn review_command_respects_enabled_flag() {
        let mut cfg = AppConfig::default();
        cfg.review.command = Some("opencode run --json".to_string());
        cfg.review.enabled = false;
        assert!(!cfg.review_enabled());
        assert!(cfg.review_command().is_none());
    }
}
