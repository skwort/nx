use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use tempfile::NamedTempFile;

use crate::{Result, SystemState};

const CACHE_SCHEMA: u32 = 1;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CacheKey<'a> {
    schema: u32,
    source_nar_hash: &'a str,
    locks: &'a Value,
    configuration: &'a str,
}

pub(crate) fn key(metadata: &Value, configuration: &str) -> Result<String> {
    let source_nar_hash = metadata
        .get("locked")
        .and_then(|locked| locked.get("narHash"))
        .and_then(Value::as_str)
        .ok_or("flake metadata does not contain a source NAR hash")?;
    let locks = metadata
        .get("locks")
        .ok_or("flake metadata does not contain a resolved lock graph")?;
    let bytes = serde_json::to_vec(&CacheKey {
        schema: CACHE_SCHEMA,
        source_nar_hash,
        locks,
        configuration,
    })?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(crate) fn load(directory: &Path, key: &str) -> Result<Option<SystemState>> {
    let path = directory.join(format!("{key}.json"));
    match fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn store(directory: &Path, key: &str, state: &SystemState) -> Result<()> {
    fs::create_dir_all(directory)?;
    let path = directory.join(format!("{key}.json"));
    let mut temporary = NamedTempFile::new_in(directory)?;
    serde_json::to_writer(temporary.as_file_mut(), state)?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::key;
    use serde_json::json;

    fn metadata(source: &str, revision: &str) -> serde_json::Value {
        json!({
            "locked": { "narHash": source },
            "locks": {
                "version": 7,
                "nodes": {
                    "root": { "inputs": { "nixpkgs": "nixpkgs" } },
                    "nixpkgs": { "locked": { "narHash": revision } }
                }
            }
        })
    }

    #[test]
    fn key_changes_with_source_inputs_and_configuration() {
        let original = metadata("sha256-source-a", "sha256-input-a");
        let changed_source = metadata("sha256-source-b", "sha256-input-a");
        let changed_input = metadata("sha256-source-a", "sha256-input-b");

        let original_key = key(&original, "alpha").unwrap();
        assert_eq!(original_key, key(&original, "alpha").unwrap());
        assert_ne!(original_key, key(&changed_source, "alpha").unwrap());
        assert_ne!(original_key, key(&changed_input, "alpha").unwrap());
        assert_ne!(original_key, key(&original, "beta").unwrap());
    }
}
