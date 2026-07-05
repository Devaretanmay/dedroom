use std::env;
use std::time::Duration;
use serde_json::json;

pub async fn try_push_runtime_env(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{}/admin/runtime-env", port);
    let client = match reqwest::Client::builder().timeout(Duration::from_millis(500)).build() {
        Ok(c) => c,
        Err(_) => return false,
    };

    // Gather explicit environment variables
    let max_repeats = env::var("DEDROOM_MAX_REPEATS").ok().and_then(|s| s.parse::<u32>().ok());
    let openai_base_url = env::var("OPENAI_BASE_URL").ok().or_else(|| env::var("DEDROOM_OPENAI_BASE_URL").ok());
    let anthropic_base_url = env::var("ANTHROPIC_BASE_URL").ok().or_else(|| env::var("DEDROOM_ANTHROPIC_BASE_URL").ok());
    let api_key = env::var("OPENAI_API_KEY").ok().or_else(|| env::var("ANTHROPIC_API_KEY").ok());
    
    // Send even if all are None, to check if proxy is alive (since it updates live)
    // Actually, we could just send what we have.
    let payload = json!({
        "max_repeats": max_repeats,
        "openai_base_url": openai_base_url,
        "anthropic_base_url": anthropic_base_url,
        "api_key": api_key,
    });

    match client.post(&url).json(&payload).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}
