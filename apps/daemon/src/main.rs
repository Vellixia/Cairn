use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let config = cairn_daemon::DaemonConfig::from_env();
    cairn_daemon::logging::init(&config)?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "cairnd starting");

    let (tx, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = tx.send(true);
    });

    cairn_daemon::run(config, rx).await
}
