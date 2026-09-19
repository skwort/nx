use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

use crate::{AppPaths, CheckReport, CheckRequest, Result};

const REPORT_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportTarget {
    pub flake: PathBuf,
    pub configuration: String,
}

#[derive(Serialize, Deserialize)]
struct StoredReport {
    schema: u32,
    target: ReportTarget,
    report: CheckReport,
}

pub fn load_latest_report(target: &ReportTarget) -> Result<Option<CheckReport>> {
    let paths = AppPaths::discover()?;
    load(&paths.cache_dir.join("reports"), target)
}

pub(crate) fn store(directory: &Path, request: &CheckRequest, report: &CheckReport) -> Result<()> {
    let target = ReportTarget {
        flake: request.flake.canonicalize()?,
        configuration: request.host.clone(),
    };
    fs::create_dir_all(directory)?;
    let path = report_path(directory, &target)?;
    let stored = StoredReport {
        schema: REPORT_SCHEMA,
        target,
        report: report.clone(),
    };
    let mut temporary = NamedTempFile::new_in(directory)?;
    serde_json::to_writer(temporary.as_file_mut(), &stored)?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn load(directory: &Path, target: &ReportTarget) -> Result<Option<CheckReport>> {
    let path = report_path(directory, target)?;
    match fs::read(path) {
        Ok(bytes) => {
            let stored: StoredReport = serde_json::from_slice(&bytes)?;
            Ok((stored.schema == REPORT_SCHEMA).then_some(stored.report))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn report_path(directory: &Path, target: &ReportTarget) -> Result<PathBuf> {
    let canonical_target = ReportTarget {
        flake: target.flake.canonicalize()?,
        configuration: target.configuration.clone(),
    };
    let bytes = serde_json::to_vec(&(REPORT_SCHEMA, canonical_target))?;
    let digest = Sha256::digest(bytes);
    let key: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(directory.join(format!("{key}.json")))
}

#[cfg(test)]
mod tests {
    use super::{ReportTarget, load, store};
    use crate::{CheckReport, CheckRequest, SystemState};
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    #[test]
    fn stores_and_loads_reports_by_target() {
        let root = TempDir::new().unwrap();
        let cache = TempDir::new().unwrap();
        let request = CheckRequest {
            flake: root.path().to_path_buf(),
            host: "alpha".to_owned(),
            offline: false,
        };
        let system = SystemState {
            kernel: "1.0".to_owned(),
            packages: BTreeMap::new(),
            inputs: BTreeMap::new(),
            system_drv: "/nix/store/system.drv".to_owned(),
        };
        let report = CheckReport {
            checked_at: 42,
            before: system.clone(),
            after: system,
        };

        store(cache.path(), &request, &report).unwrap();
        let loaded = load(
            cache.path(),
            &ReportTarget {
                flake: root.path().to_path_buf(),
                configuration: "alpha".to_owned(),
            },
        )
        .unwrap()
        .unwrap();

        assert_eq!(loaded.checked_at, 42);
    }
}
