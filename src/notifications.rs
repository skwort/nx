use base64::{Engine, engine::general_purpose::STANDARD};
use notify_rust::{Notification, Urgency};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;
use tempfile::NamedTempFile;
use tracing::{info, warn};

use crate::{CheckReport, NotificationConfig, NtfyConfig, NtfyMode, ReportTarget, Result};

type MapChange<'a, T> = (&'a str, Option<&'a T>, Option<&'a T>);

#[derive(Debug, Clone)]
pub struct UpdateNotification {
    pub fingerprint: String,
    pub body: String,
}

#[derive(Serialize)]
struct MeaningfulChanges<'a> {
    kernel: Option<(&'a str, &'a str)>,
    packages: Vec<MapChange<'a, BTreeSet<String>>>,
    inputs: Vec<MapChange<'a, String>>,
    system: Option<(&'a str, &'a str)>,
}

pub fn prepare_update_notification(report: &CheckReport) -> Result<Option<UpdateNotification>> {
    let kernel = (report.before.kernel != report.after.kernel)
        .then_some((report.before.kernel.as_str(), report.after.kernel.as_str()));
    let packages = map_changes(&report.before.packages, &report.after.packages);
    let inputs = map_changes(&report.before.inputs, &report.after.inputs);
    let system = (report.before.system_drv != report.after.system_drv).then_some((
        report.before.system_drv.as_str(),
        report.after.system_drv.as_str(),
    ));
    if kernel.is_none() && packages.is_empty() && inputs.is_empty() && system.is_none() {
        return Ok(None);
    }

    let changes = MeaningfulChanges {
        kernel,
        packages,
        inputs,
        system,
    };
    let fingerprint = Sha256::digest(serde_json::to_vec(&changes)?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let body = notification_body(&changes);
    Ok(Some(UpdateNotification { fingerprint, body }))
}

#[derive(Default, Serialize, Deserialize)]
struct DeliveryState {
    fingerprints: BTreeMap<String, String>,
}

pub struct NotificationDispatcher {
    backends: Vec<Box<dyn NotificationBackend>>,
    state_path: PathBuf,
}

trait NotificationBackend: Send {
    fn destination(&self) -> String;
    fn send(&self, update: &UpdateNotification, report: &CheckReport, test: bool) -> Result<()>;
}

struct DesktopBackend {
    view_command: Option<Vec<String>>,
}

impl NotificationBackend for DesktopBackend {
    fn destination(&self) -> String {
        "desktop".to_owned()
    }

    fn send(&self, update: &UpdateNotification, _report: &CheckReport, test: bool) -> Result<()> {
        let command = self.view_command.clone().or_else(|| {
            Some(vec![
                "xdg-terminal-exec".to_owned(),
                "--".to_owned(),
                "nx".to_owned(),
                "update".to_owned(),
                "list".to_owned(),
            ])
        });
        show_notification_with_summary(
            update,
            command,
            if test {
                "nx test notification"
            } else {
                "NixOS updates available"
            },
            test,
        )
    }
}

struct NtfyBackend {
    config: NtfyConfig,
}

impl NotificationBackend for NtfyBackend {
    fn destination(&self) -> String {
        format!(
            "ntfy:{}",
            self.config.topic_url.as_deref().unwrap_or_default()
        )
    }

    fn send(&self, update: &UpdateNotification, report: &CheckReport, _test: bool) -> Result<()> {
        send_ntfy(&self.config, update, report)
    }
}

impl NotificationDispatcher {
    pub fn new(
        config: NotificationConfig,
        target: &ReportTarget,
        cache_dir: &Path,
    ) -> Result<Self> {
        config.ntfy.validate()?;
        let mut backends: Vec<Box<dyn NotificationBackend>> = Vec::new();
        if config.ntfy.enabled {
            backends.push(Box::new(NtfyBackend {
                config: config.ntfy,
            }));
        }
        if config.enabled {
            backends.push(Box::new(DesktopBackend {
                view_command: config.view_command,
            }));
        }
        let digest = Sha256::digest(serde_json::to_vec(target)?);
        let key: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Self {
            backends,
            state_path: cache_dir.join("notifications").join(format!("{key}.json")),
        })
    }

    pub fn dispatch(&self, report: &CheckReport, test: bool) -> Result<()> {
        if test && self.backends.is_empty() {
            return Err("no notification channels are enabled".into());
        }
        let update = prepare_update_notification(report)?;
        let mut state = if test {
            DeliveryState::default()
        } else {
            self.load_state()?
        };
        if update.is_none() && !test {
            if !state.fingerprints.is_empty() {
                state.fingerprints.clear();
                self.save_state(&state)?;
            }
            return Ok(());
        }
        let update = update.unwrap_or(UpdateNotification {
            fingerprint: String::new(),
            body: "No updates in the latest cached report".to_owned(),
        });
        let mut errors = Vec::new();
        let mut state_changed = false;
        for backend in &self.backends {
            let destination = backend.destination();
            if test || state.fingerprints.get(&destination) != Some(&update.fingerprint) {
                match backend.send(&update, report, test) {
                    Ok(()) => {
                        if !test {
                            state
                                .fingerprints
                                .insert(destination, update.fingerprint.clone());
                            state_changed = true;
                        }
                    }
                    Err(error) => {
                        warn!(%destination, %error, "failed to send update notification");
                        errors.push(format!("{destination}: {error}"));
                    }
                }
            }
        }
        if state_changed {
            self.save_state(&state)?;
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; ").into())
        }
    }

    fn load_state(&self) -> Result<DeliveryState> {
        match fs::read(&self.state_path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(DeliveryState::default())
            }
            Err(error) => Err(error.into()),
        }
    }

    fn save_state(&self, state: &DeliveryState) -> Result<()> {
        let parent = self
            .state_path
            .parent()
            .ok_or("notification state has no parent")?;
        fs::create_dir_all(parent)?;
        let mut temporary = NamedTempFile::new_in(parent)?;
        serde_json::to_writer(temporary.as_file_mut(), state)?;
        temporary.as_file_mut().sync_all()?;
        temporary
            .persist(&self.state_path)
            .map_err(|error| error.error)?;
        Ok(())
    }
}

fn send_ntfy(config: &NtfyConfig, update: &UpdateNotification, report: &CheckReport) -> Result<()> {
    let url = config
        .topic_url
        .as_deref()
        .ok_or("ntfy topic URL is missing")?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;
    let mut request = client
        .post(url)
        .header("X-Title", "NixOS updates available");
    if let Some(token) = read_token(config)? {
        request = request.bearer_auth(token);
    }
    request = match config.mode {
        NtfyMode::Summary => request.body(update.body.clone()),
        NtfyMode::Detailed => request
            .header("X-Message", rfc2047(&update.body))
            .header("X-Filename", "nx-update-report.txt")
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(full_report(report)),
    };
    let response = request.send()?;
    if !response.status().is_success() {
        return Err(format!("ntfy returned HTTP {}", response.status()).into());
    }
    info!("ntfy update notification sent");
    Ok(())
}

fn read_token(config: &NtfyConfig) -> Result<Option<String>> {
    let value = if let Some(path) = &config.token_file {
        Some(fs::read_to_string(path)?)
    } else if let Some(name) = &config.token_env {
        Some(
            env::var(name)
                .map_err(|_| format!("ntfy token environment variable {name} is unavailable"))?,
        )
    } else {
        None
    };
    value
        .map(|value| {
            let token = value.trim().to_owned();
            if token.is_empty() {
                Err("ntfy token is empty".into())
            } else {
                Ok(token)
            }
        })
        .transpose()
}

fn rfc2047(value: &str) -> String {
    format!("=?UTF-8?B?{}?=", STANDARD.encode(value))
}

fn full_report(report: &CheckReport) -> String {
    let mut lines = vec![
        format!("Checked at Unix time {}", report.checked_at),
        String::new(),
        "Summary".to_owned(),
    ];
    lines.push(format!(
        "  Kernel   {} → {}",
        report.before.kernel, report.after.kernel
    ));
    lines.push(format!(
        "  System   {} → {}",
        report.before.system_drv, report.after.system_drv
    ));
    lines.push(String::new());
    lines.push("Flake inputs".to_owned());
    for (name, old, new) in map_changes(&report.before.inputs, &report.after.inputs) {
        lines.push(format!(
            "  {name}: {} → {}",
            old.map(String::as_str).unwrap_or("(none)"),
            new.map(String::as_str).unwrap_or("(none)")
        ));
    }
    lines.push(String::new());
    lines.push("Declared packages".to_owned());
    for (name, old, new) in map_changes(&report.before.packages, &report.after.packages) {
        let format_set = |items: Option<&BTreeSet<String>>| {
            items
                .map(|items| items.iter().cloned().collect::<Vec<_>>().join(", "))
                .unwrap_or_else(|| "(none)".to_owned())
        };
        lines.push(format!(
            "  {name}: {} → {}",
            format_set(old),
            format_set(new)
        ));
    }
    lines.join("\n") + "\n"
}

fn show_notification_with_summary(
    update: &UpdateNotification,
    view_command: Option<Vec<String>>,
    summary: &str,
    wait_for_action: bool,
) -> Result<()> {
    let mut notification = Notification::new();
    notification
        .appname("nx")
        .summary(summary)
        .body(&update.body)
        .icon("software-update-available")
        .urgency(Urgency::Normal);

    let view_command = view_command.filter(|command| !command.is_empty());
    if view_command.is_some() {
        notification
            .action("default", "View changes")
            .action("view", "View changes");
    }
    let handle = notification.show()?;
    info!("desktop update notification sent");

    if let Some(command) = view_command {
        let wait = move || {
            handle.wait_for_action(|action| {
                if matches!(action, "default" | "view") {
                    let mut process = Command::new(&command[0]);
                    process.args(&command[1..]);
                    if let Err(error) = process.spawn() {
                        warn!(%error, "failed to launch notification view command");
                    }
                }
            });
        };
        if wait_for_action {
            wait();
        } else {
            thread::spawn(wait);
        }
    }
    Ok(())
}

fn notification_body(changes: &MeaningfulChanges<'_>) -> String {
    let mut lines = Vec::new();
    if let Some((before, after)) = changes.kernel {
        lines.push(format!(
            "Kernel {} → {}",
            escape_markup(before),
            escape_markup(after)
        ));
    }
    if !changes.packages.is_empty() {
        lines.push(format!(
            "{} package {}",
            changes.packages.len(),
            plural(changes.packages.len(), "update", "updates")
        ));
    }
    if !changes.inputs.is_empty() {
        lines.push(format!(
            "{} flake {} updated",
            changes.inputs.len(),
            plural(changes.inputs.len(), "input", "inputs")
        ));
    }
    if lines.is_empty() && changes.system.is_some() {
        lines.push("System derivation changed".to_owned());
    }
    lines.join("\n")
}

fn map_changes<'a, T: PartialEq>(
    before: &'a BTreeMap<String, T>,
    after: &'a BTreeMap<String, T>,
) -> Vec<MapChange<'a, T>> {
    let mut names = before.keys().map(String::as_str).collect::<BTreeSet<_>>();
    names.extend(after.keys().map(String::as_str));
    names
        .into_iter()
        .filter_map(|name| {
            let old = before.get(name);
            let new = after.get(name);
            (old != new).then_some((name, old, new))
        })
        .collect()
}

fn plural<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if count == 1 { singular } else { plural }
}

fn escape_markup(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::{
        NotificationBackend, NotificationDispatcher, UpdateNotification, full_report,
        prepare_update_notification, send_ntfy,
    };
    use crate::{CheckReport, NtfyConfig, NtfyMode, SystemState};
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tempfile::TempDir;

    fn state(kernel: &str, package: &str, input: &str, system: &str) -> SystemState {
        SystemState {
            kernel: kernel.to_owned(),
            packages: BTreeMap::from([(
                "example".to_owned(),
                BTreeSet::from([package.to_owned()]),
            )]),
            inputs: BTreeMap::from([("nixpkgs".to_owned(), input.to_owned())]),
            system_drv: system.to_owned(),
        }
    }

    #[test]
    fn prepares_a_compact_update_notification() {
        let report = CheckReport {
            checked_at: 1,
            before: state("1.0", "1.0", "old", "old-system"),
            after: state("2.0", "2.0", "new", "new-system"),
        };
        let update = prepare_update_notification(&report).unwrap().unwrap();

        assert_eq!(
            update.body,
            "Kernel 1.0 → 2.0\n1 package update\n1 flake input updated"
        );
    }

    #[test]
    fn includes_raw_input_revisions_in_the_fingerprint() {
        let first = CheckReport {
            checked_at: 1,
            before: state("1.0", "1.0", "old-a", "old-system-a"),
            after: state("1.0", "1.0", "new-a", "new-system-a"),
        };
        let second = CheckReport {
            checked_at: 2,
            before: state("1.0", "1.0", "old-b", "old-system-b"),
            after: state("1.0", "1.0", "new-b", "new-system-b"),
        };

        let first = prepare_update_notification(&first).unwrap().unwrap();
        let second = prepare_update_notification(&second).unwrap().unwrap();
        assert_ne!(first.fingerprint, second.fingerprint);
    }

    #[test]
    fn skips_reports_without_updates() {
        let state = state("1.0", "1.0", "same", "same-system");
        let report = CheckReport {
            checked_at: 1,
            before: state.clone(),
            after: state,
        };

        assert!(prepare_update_notification(&report).unwrap().is_none());
    }

    struct RecordingBackend {
        name: &'static str,
        calls: Arc<AtomicUsize>,
        fail: Arc<AtomicBool>,
    }

    impl NotificationBackend for RecordingBackend {
        fn destination(&self) -> String {
            self.name.to_owned()
        }

        fn send(
            &self,
            _update: &UpdateNotification,
            _report: &CheckReport,
            _test: bool,
        ) -> crate::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail.load(Ordering::SeqCst) {
                Err("delivery failed".into())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn persists_successes_and_retries_failed_channels() {
        let directory = TempDir::new().unwrap();
        let state_path = directory.path().join("delivery.json");
        let good_calls = Arc::new(AtomicUsize::new(0));
        let bad_calls = Arc::new(AtomicUsize::new(0));
        let fail = Arc::new(AtomicBool::new(true));
        let make_dispatcher = || NotificationDispatcher {
            backends: vec![
                Box::new(RecordingBackend {
                    name: "good",
                    calls: good_calls.clone(),
                    fail: Arc::new(AtomicBool::new(false)),
                }),
                Box::new(RecordingBackend {
                    name: "bad",
                    calls: bad_calls.clone(),
                    fail: fail.clone(),
                }),
            ],
            state_path: state_path.clone(),
        };
        let report = CheckReport {
            checked_at: 1,
            before: state("1", "1", "old", "old-system"),
            after: state("2", "2", "new", "new-system"),
        };
        assert!(make_dispatcher().dispatch(&report, false).is_err());
        fail.store(false, Ordering::SeqCst);
        make_dispatcher().dispatch(&report, false).unwrap();
        make_dispatcher().dispatch(&report, false).unwrap();
        assert_eq!(good_calls.load(Ordering::SeqCst), 1);
        assert_eq!(bad_calls.load(Ordering::SeqCst), 2);
        make_dispatcher().dispatch(&report, true).unwrap();
        assert_eq!(good_calls.load(Ordering::SeqCst), 2);
        let unchanged = CheckReport {
            before: report.before.clone(),
            after: report.before.clone(),
            ..report.clone()
        };
        make_dispatcher().dispatch(&unchanged, false).unwrap();
        make_dispatcher().dispatch(&report, false).unwrap();
        assert_eq!(good_calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn sends_summary_and_detailed_ntfy_requests() {
        let report = CheckReport {
            checked_at: 1,
            before: state("1", "1", "old", "old-system"),
            after: state("2", "2", "new", "new-system"),
        };
        let update = prepare_update_notification(&report).unwrap().unwrap();
        let secret = TempDir::new().unwrap();
        let token_file = secret.path().join("token");
        std::fs::write(&token_file, "example-token\n").unwrap();
        for mode in [NtfyMode::Summary, NtfyMode::Detailed] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/updates", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut headers = String::new();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if line.to_ascii_lowercase().starts_with("content-length:") {
                        length = line.split(':').nth(1).unwrap().trim().parse().unwrap();
                    }
                    headers.push_str(&line);
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
                (
                    headers.to_ascii_lowercase(),
                    String::from_utf8(body).unwrap(),
                )
            });
            let config = NtfyConfig {
                enabled: true,
                topic_url: Some(url),
                mode,
                token_file: Some(token_file.clone()),
                token_env: None,
            };
            send_ntfy(&config, &update, &report).unwrap();
            let (headers, body) = server.join().unwrap();
            assert!(headers.contains("post /updates http/1.1"));
            assert!(headers.contains("authorization: bearer example-token"));
            match mode {
                NtfyMode::Summary => assert_eq!(body, update.body),
                NtfyMode::Detailed => {
                    assert_eq!(body, full_report(&report));
                    assert!(headers.contains("x-filename: nx-update-report.txt"));
                    assert!(headers.contains("x-message: =?utf-8?b?"));
                }
            }
        }
    }

    #[test]
    #[ignore = "requires NX_LIVE_NTFY_URL and NX_LIVE_NTFY_TOKEN"]
    fn sends_to_live_ntfy() {
        let url = std::env::var("NX_LIVE_NTFY_URL").unwrap();
        let report = CheckReport {
            checked_at: 1,
            before: state("test-1", "test-1", "test-old", "test-old-system"),
            after: state("test-2", "test-2", "test-new", "test-new-system"),
        };
        let update = prepare_update_notification(&report).unwrap().unwrap();
        for mode in [NtfyMode::Summary, NtfyMode::Detailed] {
            send_ntfy(
                &NtfyConfig {
                    enabled: true,
                    topic_url: Some(url.clone()),
                    mode,
                    token_file: None,
                    token_env: Some("NX_LIVE_NTFY_TOKEN".to_owned()),
                },
                &update,
                &report,
            )
            .unwrap();
        }
    }
}
