use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

mod cache;
mod config;

pub use config::{AppPaths, Config};

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckRequest {
    pub flake: PathBuf,
    pub host: String,
    #[serde(default)]
    pub offline: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckReport {
    pub before: SystemState,
    pub after: SystemState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemState {
    pub kernel: String,
    pub packages: BTreeMap<String, BTreeSet<String>>,
    pub inputs: BTreeMap<String, String>,
    pub system_drv: String,
}

pub fn check(request: &CheckRequest) -> Result<CheckReport> {
    let source = request.flake.canonicalize()?;
    if !source.join("flake.nix").is_file() {
        return Err(format!("{} does not contain flake.nix", source.display()).into());
    }

    let temp = TempDir::new()?;
    let before_dir = temp.path().join("before");
    let after_dir = temp.path().join("after");
    copy_tree(&source, &before_dir)?;
    copy_tree(&source, &after_dir)?;
    nix_flake_update(&after_dir, request.offline)?;

    let cache_dir = AppPaths::discover()?.cache_dir.join("systems");
    let before = evaluate(&before_dir, &request.host, request.offline, &cache_dir)?;
    let after = evaluate(&after_dir, &request.host, request.offline, &cache_dir)?;
    Ok(CheckReport { before, after })
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    let output = Command::new("cp")
        .args([
            "-a",
            &format!("{}/.", source.display()),
            &destination.display().to_string(),
        ])
        .output()?;
    ensure_success(output, "copy snapshot").map(|_| ())
}

fn nix_flake_update(path: &Path, offline: bool) -> Result<()> {
    let mut command = Command::new("nix");
    add_common_args(&mut command, offline);
    let output = command
        .args(["flake", "update", "--flake"])
        .arg(format!("path:{}", path.display()))
        .output()?;
    ensure_success(output, "update temporary flake").map(|_| ())
}

fn evaluate(path: &Path, host: &str, offline: bool, cache_dir: &Path) -> Result<SystemState> {
    let metadata_raw = nix_metadata(path, offline)?;
    let metadata: Value = serde_json::from_str(&metadata_raw)?;
    let cache_key = cache::key(&metadata, host)?;
    if let Some(state) = cache::load(cache_dir, &cache_key)? {
        return Ok(state);
    }

    let target = format!("path:{}#nixosConfigurations.{host}.config", path.display());
    let system_drv = nix_eval(&target, ".system.build.toplevel.drvPath", "--raw", offline)?;
    let kernel = nix_eval(
        &target,
        ".boot.kernelPackages.kernel.version",
        "--raw",
        offline,
    )?;
    let packages = nix_eval_packages(&target, offline)?;
    let state = SystemState {
        kernel,
        packages: parse_declared_packages(&packages)?,
        inputs: parse_inputs(&metadata),
        system_drv,
    };
    cache::store(cache_dir, &cache_key, &state)?;
    Ok(state)
}

fn nix_eval(target: &str, attribute: &str, mode: &str, offline: bool) -> Result<String> {
    let mut command = Command::new("nix");
    add_common_args(&mut command, offline);
    let output = command
        .args([
            "eval",
            "--no-write-lock-file",
            "--option",
            "allow-import-from-derivation",
            "false",
            mode,
        ])
        .arg(format!("{target}{attribute}"))
        .output()?;
    String::from_utf8(ensure_success(output, "evaluate configuration")?)
        .map_err(|error| error.into())
}

fn nix_eval_packages(target: &str, offline: bool) -> Result<String> {
    let mut command = Command::new("nix");
    add_common_args(&mut command, offline);
    let output = command
        .args([
            "eval",
            "--no-write-lock-file",
            "--option",
            "allow-import-from-derivation",
            "false",
            "--json",
        ])
        .arg(format!("{target}.environment.systemPackages"))
        .args([
            "--apply",
            "ps: map (p: { name = p.name or null; pname = p.pname or null; version = p.version or null; outputPath = p.outPath; }) ps",
        ])
        .output()?;
    String::from_utf8(ensure_success(output, "evaluate declared packages")?)
        .map_err(|error| error.into())
}

fn nix_metadata(path: &Path, offline: bool) -> Result<String> {
    let mut command = Command::new("nix");
    add_common_args(&mut command, offline);
    let output = command
        .args(["flake", "metadata", "--json", "--no-write-lock-file"])
        .arg(format!("path:{}", path.display()))
        .output()?;
    String::from_utf8(ensure_success(output, "read flake metadata")?).map_err(|error| error.into())
}

fn add_common_args(command: &mut Command, offline: bool) {
    if offline {
        command.arg("--offline");
    }
}

fn ensure_success(output: Output, operation: &str) -> Result<Vec<u8>> {
    if output.status.success() {
        return Ok(output.stdout);
    }
    let details = String::from_utf8_lossy(&output.stderr);
    Err(format!("{operation} failed: {}", details.trim()).into())
}

pub fn parse_declared_packages(raw: &str) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let values: Vec<Value> = serde_json::from_str(raw)?;
    let mut packages = BTreeMap::new();

    for package in values {
        let name = package["pname"]
            .as_str()
            .or_else(|| package["name"].as_str())
            .unwrap_or("<unnamed>");
        let version = package["version"].as_str().unwrap_or("<unknown>");
        packages
            .entry(name.to_owned())
            .or_insert_with(BTreeSet::new)
            .insert(version.to_owned());
    }

    Ok(packages)
}

fn parse_inputs(metadata: &Value) -> BTreeMap<String, String> {
    let mut inputs = BTreeMap::new();
    let Some(nodes) = metadata
        .get("locks")
        .cloned()
        .and_then(|locks| locks.get("nodes").cloned())
        .and_then(|nodes| nodes.as_object().cloned())
    else {
        return inputs;
    };

    for (name, node) in nodes {
        if name == "root" {
            continue;
        }
        if let Some(locked) = node.get("locked") {
            let revision = locked
                .get("rev")
                .and_then(Value::as_str)
                .or_else(|| locked.get("narHash").and_then(Value::as_str));
            if let Some(revision) = revision {
                inputs.insert(name, revision.to_owned());
            }
        }
    }
    inputs
}
