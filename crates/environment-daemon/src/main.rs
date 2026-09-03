use clap::Parser as _;
use environment_daemon::{DaemonRuntime, config::DaemonArgs, server};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    // Workspace builds compile both rustls providers (ring via object_store,
    // aws-lc-rs via our own deps); pick one before any TLS client exists.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let config = DaemonArgs::parse().into_config()?;
    let runtime = DaemonRuntime::new(config)?;
    server::run(runtime).await
}
