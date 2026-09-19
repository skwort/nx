use std::io::IsTerminal;
use tracing_subscriber::EnvFilter;

use crate::Result;

#[derive(Debug, Clone, Copy)]
pub enum LogMode {
    Cli { verbosity: u8 },
    Daemon,
}

pub fn init_logging(mode: LogMode) -> Result<()> {
    let default_filter = match mode {
        LogMode::Cli { verbosity: 0 } => "nx=warn",
        LogMode::Cli { verbosity: 1 } => "nx=info",
        LogMode::Cli { verbosity: 2 } => "nx=debug",
        LogMode::Cli { .. } => "nx=trace",
        LogMode::Daemon => "nx=info",
    };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));
    let ansi = matches!(mode, LogMode::Cli { .. }) && std::io::stderr().is_terminal();

    tracing_subscriber::fmt()
        .compact()
        .with_ansi(ansi)
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init()
        .map_err(|error| format!("failed to install logging subscriber: {error}"))?;
    Ok(())
}
