use crate::App;
use once_cell::sync::OnceCell;
use std::error::Error;
use std::process::Command;
use std::sync::Mutex;

static ISSUES_CACHE: OnceCell<Mutex<Option<String>>> = OnceCell::new();

// Example GitHub issues JSON output:
/*
[
  {
    "body": "This is a body of the GH issue.",
    "labels": [
      {
        "id": "LA_kwDOOTdaS88AAAAB9JPIwX",
        "name": "bug",
        "description": "Something isn't working",
        "color": "d73a4a"
      }
    ],
    "number": 42,
    "title": "This is a title of the GH issue."
  }
]
*/
pub fn github_list_issues(app: &mut App) -> Result<String, Box<dyn Error>> {
    // Initialize cache if not already initialized
    let cache = ISSUES_CACHE.get_or_init(|| Mutex::new(None));
    let mut cache = cache.lock().unwrap();

    // Return cached data if available
    if let Some(cached_data) = cache.as_ref() {
        app.add_log("INFO", "Using cached GitHub issues");
        return Ok(cached_data.clone());
    }

    // Cache miss - fetch from GitHub
    let output = Command::new("gh")
        .args(["issue", "list", "--json", "number,title,labels,body"])
        .output()?;

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        // Handle specific error cases
        if stderr.contains("has disabled issues") {
            app.add_log("WARN", "Repository has issues disabled");
            let empty_list = "[]".to_string();
            *cache = Some(empty_list.clone());
            return Ok(empty_list);
        }
        app.add_error(stderr);
        return Err("Failed to list issues".into());
    }

    let json_str = String::from_utf8(output.stdout)?;
    app.add_log("INFO", "Successfully retrieved fresh GitHub issues");

    // Update cache
    *cache = Some(json_str.clone());

    Ok(json_str)
}
