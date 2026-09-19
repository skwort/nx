use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::Result;

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_file: PathBuf,
    pub cache_dir: PathBuf,
    pub socket: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let dirs = ProjectDirs::from("", "", "nx")
            .ok_or("platform application directories are unavailable")?;
        let runtime_dir = dirs
            .runtime_dir()
            .ok_or("platform runtime directory is unavailable")?;

        Ok(Self {
            config_file: dirs.config_dir().join("config.toml"),
            cache_dir: dirs.cache_dir().to_path_buf(),
            socket: runtime_dir.join("nxd.sock"),
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub system: SystemConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SystemConfig {
    pub flake: Option<PathBuf>,
    pub configuration: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub socket: Option<PathBuf>,
    pub check_interval: Option<String>,
}

impl DaemonConfig {
    pub fn interval(&self) -> Result<Option<Duration>> {
        self.check_interval
            .as_deref()
            .map(parse_duration)
            .transpose()
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(contents) => Ok(toml::from_str(&contents)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }
}

fn parse_duration(value: &str) -> Result<Duration> {
    let value = value.trim();
    let split = value
        .find(|character: char| !character.is_ascii_digit())
        .ok_or("check_interval requires a unit: s, m, h, or d")?;
    let (amount, unit) = value.split_at(split);
    let amount: u64 = amount.parse()?;
    if amount == 0 {
        return Err("check_interval must be greater than zero".into());
    }
    let seconds = match unit {
        "s" => Some(amount),
        "m" => amount.checked_mul(60),
        "h" => amount.checked_mul(60 * 60),
        "d" => amount.checked_mul(24 * 60 * 60),
        _ => None,
    }
    .ok_or("invalid check_interval; use a number followed by s, m, h, or d")?;
    Ok(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::parse_duration;
    use std::time::Duration;

    #[test]
    fn parses_check_intervals() {
        assert_eq!(parse_duration("30m").unwrap(), Duration::from_secs(1_800));
        assert_eq!(parse_duration("6h").unwrap(), Duration::from_secs(21_600));
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86_400));
    }

    #[test]
    fn rejects_invalid_check_intervals() {
        for value in ["0h", "6", "soon", "1 hour"] {
            assert!(parse_duration(value).is_err(), "accepted {value}");
        }
    }
}
