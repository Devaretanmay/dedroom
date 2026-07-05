use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT, ACCEPT};
use serde_json::Value;
use std::path::PathBuf;

pub fn get_github_oauth_token() -> Result<String> {
    // 1. Try env var
    if let Ok(token) = std::env::var("GITHUB_TOKEN").or_else(|_| std::env::var("GH_TOKEN")) {
        return Ok(token);
    }
    if let Ok(token) = std::env::var("GITHUB_COPILOT_GITHUB_TOKEN") {
        return Ok(token);
    }

    // 2. Try github-copilot apps.json and hosts.json
    let config_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~")).join(".config");
    let copilot_config_dir = config_dir.join("github-copilot");
    
    let paths = [
        copilot_config_dir.join("apps.json"),
        copilot_config_dir.join("hosts.json"),
    ];

    for path in paths {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(json) = serde_json::from_str::<Value>(&content) {
                if let Some(token) = extract_oauth_token(&json) {
                    return Ok(token);
                }
            }
        }
    }

    anyhow::bail!("No GitHub Copilot OAuth token found. Please run `gh auth login` or install Copilot.")
}

fn extract_oauth_token(json: &Value) -> Option<String> {
    if let Value::Object(map) = json {
        let keys = ["oauth_token", "oauthToken", "token", "access_token", "accessToken"];
        for key in keys {
            if let Some(Value::String(val)) = map.get(key) {
                return Some(val.clone());
            }
        }
        for value in map.values() {
            if let Some(val) = extract_oauth_token(value) {
                return Some(val);
            }
        }
    }
    None
}

pub async fn exchange_for_copilot_token(oauth_token: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_str(&format!("token {}", oauth_token))?);
    headers.insert(USER_AGENT, HeaderValue::from_static("GitHubCopilotChat/0.35.0"));
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert("editor-version", HeaderValue::from_static("vscode/1.107.0"));
    headers.insert("editor-plugin-version", HeaderValue::from_static("copilot-chat/0.35.0"));

    let resp = client
        .get("https://api.github.com/copilot_internal/v2/token")
        .headers(headers)
        .send()
        .await
        .context("Failed to contact GitHub Copilot token endpoint")?;

    if !resp.status().is_success() {
        anyhow::bail!("Failed to exchange token. Status: {}", resp.status());
    }

    let body: Value = resp.json().await.context("Failed to parse token response")?;
    
    if let Some(token) = body.get("token").and_then(|v| v.as_str()) {
        Ok(token.to_string())
    } else {
        anyhow::bail!("No 'token' field in Copilot response")
    }
}
