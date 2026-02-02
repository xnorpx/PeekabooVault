//! PeekabooVault NVR - Main Entry Point

use clap::Parser;
use peeka_boo_vault::{Config, run_server};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;
use tracing::{Level, info};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[derive(Parser, Debug)]
#[command(name = "peekaboovault")]
#[command(
    author,
    version,
    about = "PeekabooVault NVR - Camera Discovery and Recording"
)]
struct Args {
    /// Path to configuration file
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,

    /// Port to bind the HTTP server (overrides config)
    #[arg(short, long)]
    port: Option<u16>,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse command line arguments
    let args = Args::parse();

    // Set up logging
    let filter = if args.verbose {
        EnvFilter::from_default_env()
            .add_directive(Level::DEBUG.into())
            .add_directive("peekaboovault=debug".parse()?)
    } else {
        EnvFilter::from_default_env()
            .add_directive(Level::INFO.into())
            .add_directive("peekaboovault=info".parse()?)
    };

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .init();

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "Starting PeekabooVault NVR"
    );

    // Load configuration
    let config_path = if args.config.exists() {
        Some(&args.config)
    } else {
        info!("Config file not found, using defaults");
        None
    };
    let mut config = Config::load_or_default(config_path);

    // Override port if specified
    if let Some(port) = args.port {
        config.server.bind_address.set_port(port);
    }

    info!(?config, "Loaded configuration");

    // Create cancellation token for graceful shutdown
    let cancellation_token = CancellationToken::new();

    // Set up signal handlers
    let shutdown_token = cancellation_token.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
        info!("Received Ctrl+C, initiating shutdown");
        shutdown_token.cancel();
    });

    // Run the server
    run_server(config, cancellation_token).await?;

    info!("PeekabooVault shutdown complete");
    Ok(())
}
