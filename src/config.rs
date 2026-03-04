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
/// ```
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub ai: AiConfig,
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
    /// - API key:   `AUTOPR_API_KEY` > `ANTHROPIC_API_KEY` (anthropic) / `OPENAI_KEY` or `OPENAI_API_KEY` (openai)
    /// - Model:     `AUTOPR_MODEL` > `OPENAI_MODEL` (backward compat)
    /// - Base URL:  `AUTOPR_BASE_URL` > `OPENAI_BASE_URL` (backward compat)
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

        if let Ok(v) = std::env::var("AUTOPR_MODEL").or_else(|_| std::env::var("OPENAI_MODEL")) {
            self.ai.model = Some(v);
        }

        if let Ok(v) =
            std::env::var("AUTOPR_BASE_URL").or_else(|_| std::env::var("OPENAI_BASE_URL"))
        {
            self.ai.base_url = Some(v);
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
}
