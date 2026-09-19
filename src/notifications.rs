use notify_rust::{Notification, Urgency};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;
use std::thread;
use tracing::{info, warn};

use crate::{CheckReport, Result};

type MapChange<'a, T> = (&'a str, Option<&'a T>, Option<&'a T>);

#[derive(Debug)]
pub struct UpdateNotification {
    pub fingerprint: String,
    pub body: String,
}

#[derive(Serialize)]
struct MeaningfulChanges<'a> {
    kernel: Option<(&'a str, &'a str)>,
    packages: Vec<MapChange<'a, BTreeSet<String>>>,
    inputs: Vec<(&'a str, ChangeKind)>,
    system: bool,
}

#[derive(Clone, Copy, Serialize)]
enum ChangeKind {
    Added,
    Changed,
    Removed,
}

pub fn prepare_update_notification(report: &CheckReport) -> Result<Option<UpdateNotification>> {
    let kernel = (report.before.kernel != report.after.kernel)
        .then_some((report.before.kernel.as_str(), report.after.kernel.as_str()));
    let packages = map_changes(&report.before.packages, &report.after.packages);
    let inputs = map_changes(&report.before.inputs, &report.after.inputs)
        .into_iter()
        .map(|(name, old, new)| (name, change_kind(old, new)))
        .collect::<Vec<_>>();
    let system = report.before.system_drv != report.after.system_drv;
    if kernel.is_none() && packages.is_empty() && inputs.is_empty() && !system {
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

pub fn show_notification(
    update: &UpdateNotification,
    view_command: Option<Vec<String>>,
) -> Result<()> {
    let mut notification = Notification::new();
    notification
        .appname("nx")
        .summary("NixOS updates available")
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
        thread::spawn(move || {
            handle.wait_for_action(|action| {
                if matches!(action, "default" | "view") {
                    let mut process = Command::new(&command[0]);
                    process.args(&command[1..]);
                    if let Err(error) = process.spawn() {
                        warn!(%error, "failed to launch notification view command");
                    }
                }
            });
        });
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
    if lines.is_empty() && changes.system {
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

fn change_kind<T>(old: Option<&T>, new: Option<&T>) -> ChangeKind {
    match (old, new) {
        (None, Some(_)) => ChangeKind::Added,
        (Some(_), None) => ChangeKind::Removed,
        (Some(_), Some(_)) => ChangeKind::Changed,
        (None, None) => unreachable!(),
    }
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
    use super::prepare_update_notification;
    use crate::{CheckReport, SystemState};
    use std::collections::{BTreeMap, BTreeSet};

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
    fn ignores_raw_input_revisions_in_the_fingerprint() {
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
        assert_eq!(first.fingerprint, second.fingerprint);
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
}
