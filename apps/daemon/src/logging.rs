//! T055-adjacent: structured JSON logs with a strict field policy — never log
//! file contents, diffs, resume tokens, or environment values (constitution).

use anyhow::Result;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

use crate::config::DaemonConfig;

pub fn init(config: &DaemonConfig) -> Result<()> {
    std::fs::create_dir_all(&config.data_dir)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(config.data_dir.join("cairnd.log"))?;

    let filter = EnvFilter::try_from_env("CAIRN_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let json_layer = fmt::layer().json().with_writer(file);

    if config.foreground {
        tracing_subscriber::registry()
            .with(filter)
            .with(json_layer)
            .with(fmt::layer().with_writer(std::io::stderr))
            .try_init()
            .ok();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(json_layer)
            .try_init()
            .ok();
    }
    Ok(())
}
