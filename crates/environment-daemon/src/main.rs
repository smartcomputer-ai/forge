use clap::Parser as _;
use environment_daemon::{DaemonRuntime, config::DaemonArgs, server};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let args = DaemonArgs::parse();
    if args.print_build {
        println!("{}", serde_json::to_string(&environment_daemon::build_info())?);
        return Ok(());
    }
    let config = args.into_config()?;
    let runtime = DaemonRuntime::new(config)?;
    server::run(runtime).await
}
