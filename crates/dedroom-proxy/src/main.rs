mod handlers;
mod intercept;
mod proxy;

use std::net::SocketAddr;
use std::path::PathBuf;

use dedroom_core::config::DedrooMConfig;
use tracing_subscriber::EnvFilter;

/// Compile-time check: AppState must be Send + Sync for axum handlers.
const _: fn() = || {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<proxy::AppState>();
    assert_sync::<proxy::AppState>();
};

fn parse_args() -> (u16, PathBuf) {
    let args: Vec<String> = std::env::args().collect();
    let mut port = 8080u16;
    let mut config_path = PathBuf::from("dedroom.yaml");

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                i += 1;
                if i < args.len() {
                    port = args[i].parse().unwrap_or(8080);
                }
            }
            "--config" => {
                i += 1;
                if i < args.len() {
                    config_path = PathBuf::from(&args[i]);
                }
            }
            _ => {}
        }
        i += 1;
    }

    (port, config_path)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing subscriber
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    // Parse CLI args
    let (port, config_path) = parse_args();

    tracing::info!(
        "DedrooM proxy starting — config: {}, port: {}",
        config_path.display(),
        port
    );

    // Load DedrooMConfig from YAML file
    let config = if config_path.exists() {
        DedrooMConfig::from_yaml_path(&config_path)?
    } else {
        tracing::warn!(
            "Config file not found at {}, using defaults",
            config_path.display()
        );
        DedrooMConfig::default()
    };

    // Build Pipeline and proxy state
    let state = proxy::AppState::new(config);

    // Build router
    let router = proxy::ProxyRouter::new(state).build();

    // Start axum server
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;

    Ok(())
}
