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
    #[serde(default)]
    pub notifications: NotificationConfig,
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
    pub startup_delay: Option<String>,
}

impl DaemonConfig {
    pub fn interval(&self) -> Result<Option<Duration>> {
        self.check_interval
            .as_deref()
            .map(|value| parse_duration("check_interval", value))
            .transpose()
    }

    pub fn startup_delay(&self) -> Result<Duration> {
        self.startup_delay
            .as_deref()
            .map(|value| parse_duration("startup_delay", value))
            .transpose()
            .map(|duration| duration.unwrap_or(Duration::from_secs(60)))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationConfig {
    #[serde(default = "notifications_enabled")]
    pub enabled: bool,
    pub view_command: Option<Vec<String>>,
    #[serde(default)]
    pub ntfy: NtfyConfig,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            view_command: None,
            ntfy: NtfyConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NtfyConfig {
    #[serde(default)]
    pub enabled: bool,
    pub topic_url: Option<String>,
    #[serde(default)]
    pub mode: NtfyMode,
    pub token_file: Option<PathBuf>,
    pub token_env: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NtfyMode {
    #[default]
    Summary,
    Detailed,
}

impl NtfyConfig {
    pub fn validate(&self) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let url = self
            .topic_url
            .as_deref()
            .ok_or("notifications.ntfy.topic_url is required")?;
        let parsed = reqwest::Url::parse(url)?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || parsed.path() == "/"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err("notifications.ntfy.topic_url must be an HTTP(S) topic URL without query or fragment".into());
        }
        if self.token_file.is_some() && self.token_env.is_some() {
            return Err("configure only one of notifications.ntfy.token_file or token_env".into());
        }
        if self
            .token_file
            .as_ref()
            .is_some_and(|path| !path.is_absolute())
        {
            return Err("notifications.ntfy.token_file must be an absolute path".into());
        }
        if self.token_env.as_deref().is_some_and(str::is_empty) {
            return Err("notifications.ntfy.token_env cannot be empty".into());
        }
        Ok(())
    }
}

fn notifications_enabled() -> bool {
    true
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

fn parse_duration(field: &str, value: &str) -> Result<Duration> {
    let value = value.trim();
    let split = value
        .find(|character: char| !character.is_ascii_digit())
        .ok_or_else(|| format!("{field} requires a unit: s, m, h, or d"))?;
    let (amount, unit) = value.split_at(split);
    let amount: u64 = amount.parse()?;
    if amount == 0 {
        return Err(format!("{field} must be greater than zero").into());
    }
    let seconds = match unit {
        "s" => Some(amount),
        "m" => amount.checked_mul(60),
        "h" => amount.checked_mul(60 * 60),
        "d" => amount.checked_mul(24 * 60 * 60),
        _ => None,
    }
    .ok_or_else(|| format!("invalid {field}; use a number followed by s, m, h, or d"))?;
    Ok(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::parse_duration;
    use std::time::Duration;

    #[test]
    fn parses_check_intervals() {
        assert_eq!(
            parse_duration("test", "30m").unwrap(),
            Duration::from_secs(1_800)
        );
        assert_eq!(
            parse_duration("test", "6h").unwrap(),
            Duration::from_secs(21_600)
        );
        assert_eq!(
            parse_duration("test", "1d").unwrap(),
            Duration::from_secs(86_400)
        );
    }

    #[test]
    fn rejects_invalid_check_intervals() {
        for value in ["0h", "6", "soon", "1 hour"] {
            assert!(parse_duration("test", value).is_err(), "accepted {value}");
        }
    }
}
