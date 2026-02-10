use crate::AttractorError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_FILE_BACKING_THRESHOLD_BYTES: usize = 64 * 1024;
const ARTIFACT_REFERENCE_PREFIX: &str = "artifact://";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtifactInfo {
    pub id: String,
    pub name: String,
    pub size_bytes: usize,
    pub stored_at: String,
    pub is_file_backed: bool,
    pub reference: String,
}

#[derive(Clone, Debug)]
enum ArtifactStorage {
    Inline(Value),
    File(PathBuf),
}

#[derive(Clone, Debug)]
struct ArtifactEntry {
    info: ArtifactInfo,
    storage: ArtifactStorage,
}

#[derive(Clone)]
pub struct ArtifactStore {
    base_dir: Option<PathBuf>,
    file_backing_threshold_bytes: usize,
    entries: Arc<RwLock<BTreeMap<String, ArtifactEntry>>>,
}

impl ArtifactStore {
    pub fn new(base_dir: Option<PathBuf>, file_backing_threshold_bytes: usize) -> Result<Self, AttractorError> {
        let threshold = if file_backing_threshold_bytes == 0 {
            DEFAULT_FILE_BACKING_THRESHOLD_BYTES
        } else {
            file_backing_threshold_bytes
        };

        if let Some(root) = base_dir.as_ref() {
            fs::create_dir_all(Self::artifacts_dir(root)).map_err(|error| {
                AttractorError::Runtime(format!(
                    "failed to create artifact directory '{}': {}",
                    Self::artifacts_dir(root).display(),
                    error
                ))
            })?;
        }

        Ok(Self {
            base_dir,
            file_backing_threshold_bytes: threshold,
            entries: Arc::new(RwLock::new(BTreeMap::new())),
        })
    }

    pub fn store_json(
        &self,
        artifact_id: impl Into<String>,
        name: impl Into<String>,
        data: &Value,
    ) -> Result<ArtifactInfo, AttractorError> {
        let artifact_id = artifact_id.into();
        validate_artifact_id(&artifact_id)?;

        let name = name.into();
        if name.trim().is_empty() {
            return Err(AttractorError::Runtime(
                "artifact name cannot be empty".to_string(),
            ));
        }

        let serialized = serde_json::to_vec(data).map_err(|error| {
            AttractorError::Runtime(format!(
                "failed to serialize artifact '{}' payload: {}",
                artifact_id, error
            ))
        })?;
        let size_bytes = serialized.len();
        let should_file_back = self.base_dir.is_some() && size_bytes > self.file_backing_threshold_bytes;

        let storage = if should_file_back {
            let file_path = self.file_path_for_id(&artifact_id)?;
            fs::write(&file_path, serialized).map_err(|error| {
                AttractorError::Runtime(format!(
                    "failed writing artifact '{}' to '{}': {}",
                    artifact_id,
                    file_path.display(),
                    error
                ))
            })?;
            ArtifactStorage::File(file_path)
        } else {
            ArtifactStorage::Inline(data.clone())
        };

        let info = ArtifactInfo {
            id: artifact_id.clone(),
            name,
            size_bytes,
            stored_at: timestamp_now(),
            is_file_backed: should_file_back,
            reference: artifact_reference(&artifact_id),
        };

        let mut entries = self
            .entries
            .write()
            .map_err(|_| AttractorError::Runtime("artifact write lock poisoned".to_string()))?;
        entries.insert(
            artifact_id,
            ArtifactEntry {
                info: info.clone(),
                storage,
            },
        );

        Ok(info)
    }

    pub fn retrieve_json(&self, artifact_id: &str) -> Result<Value, AttractorError> {
        let entry = {
            let entries = self
                .entries
                .read()
                .map_err(|_| AttractorError::Runtime("artifact read lock poisoned".to_string()))?;
            entries.get(artifact_id).cloned()
        }
        .ok_or_else(|| {
            AttractorError::Runtime(format!("artifact '{}' not found", artifact_id))
        })?;

        match entry.storage {
            ArtifactStorage::Inline(value) => Ok(value),
            ArtifactStorage::File(path) => read_json_file(&path, artifact_id),
        }
    }

    pub fn retrieve_json_by_reference(&self, reference: &str) -> Result<Value, AttractorError> {
        let artifact_id = reference
            .strip_prefix(ARTIFACT_REFERENCE_PREFIX)
            .ok_or_else(|| {
                AttractorError::Runtime(format!(
                    "artifact reference '{}' must use '{}' prefix",
                    reference, ARTIFACT_REFERENCE_PREFIX
                ))
            })?;
        self.retrieve_json(artifact_id)
    }

    pub fn has(&self, artifact_id: &str) -> bool {
        self.entries
            .read()
            .map(|entries| entries.contains_key(artifact_id))
            .unwrap_or(false)
    }

    pub fn list(&self) -> Result<Vec<ArtifactInfo>, AttractorError> {
        let entries = self
            .entries
            .read()
            .map_err(|_| AttractorError::Runtime("artifact read lock poisoned".to_string()))?;
        Ok(entries.values().map(|entry| entry.info.clone()).collect())
    }

    pub fn remove(&self, artifact_id: &str) -> Result<(), AttractorError> {
        let removed = {
            let mut entries = self
                .entries
                .write()
                .map_err(|_| AttractorError::Runtime("artifact write lock poisoned".to_string()))?;
            entries.remove(artifact_id)
        };

        if let Some(entry) = removed {
            if let ArtifactStorage::File(path) = entry.storage {
                if path.exists() {
                    fs::remove_file(&path).map_err(|error| {
                        AttractorError::Runtime(format!(
                            "failed to remove artifact file '{}': {}",
                            path.display(),
                            error
                        ))
                    })?;
                }
            }
        }

        Ok(())
    }

    pub fn clear(&self) -> Result<(), AttractorError> {
        let ids = {
            let entries = self
                .entries
                .read()
                .map_err(|_| AttractorError::Runtime("artifact read lock poisoned".to_string()))?;
            entries.keys().cloned().collect::<Vec<_>>()
        };
        for artifact_id in ids {
            self.remove(&artifact_id)?;
        }
        Ok(())
    }

    fn artifacts_dir(root: &Path) -> PathBuf {
        root.join("artifacts")
    }

    fn file_path_for_id(&self, artifact_id: &str) -> Result<PathBuf, AttractorError> {
        let Some(root) = self.base_dir.as_ref() else {
            return Err(AttractorError::Runtime(
                "artifact base_dir is not configured".to_string(),
            ));
        };
        Ok(Self::artifacts_dir(root).join(format!("{artifact_id}.json")))
    }
}

pub fn artifact_reference(artifact_id: &str) -> String {
    format!("{ARTIFACT_REFERENCE_PREFIX}{artifact_id}")
}

fn read_json_file(path: &Path, artifact_id: &str) -> Result<Value, AttractorError> {
    let bytes = fs::read(path).map_err(|error| {
        AttractorError::Runtime(format!(
            "failed reading artifact '{}' from '{}': {}",
            artifact_id,
            path.display(),
            error
        ))
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        AttractorError::Runtime(format!(
            "failed to deserialize artifact '{}' from '{}': {}",
            artifact_id,
            path.display(),
            error
        ))
    })
}

fn validate_artifact_id(artifact_id: &str) -> Result<(), AttractorError> {
    if artifact_id.is_empty() {
        return Err(AttractorError::Runtime(
            "artifact id cannot be empty".to_string(),
        ));
    }
    if artifact_id
        .chars()
        .any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'))
    {
        return Err(AttractorError::Runtime(format!(
            "artifact id '{}' contains unsupported characters",
            artifact_id
        )));
    }
    Ok(())
}

fn timestamp_now() -> String {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "{}.{:03}Z",
        since_epoch.as_secs(),
        since_epoch.subsec_millis()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn store_small_artifact_in_memory() {
        let store = ArtifactStore::new(None, 1024).expect("store should initialize");

        let info = store
            .store_json("summary", "Summary", &json!({"result": "ok"}))
            .expect("store artifact should succeed");

        assert!(!info.is_file_backed);
        assert_eq!(info.reference, "artifact://summary");
        assert_eq!(
            store
                .retrieve_json_by_reference(&info.reference)
                .expect("retrieve by ref should succeed"),
            json!({"result": "ok"})
        );
    }

    #[test]
    fn store_large_artifact_on_disk_with_stable_reference() {
        let temp = TempDir::new().expect("temp dir should create");
        let store = ArtifactStore::new(Some(temp.path().to_path_buf()), 64)
            .expect("store should initialize");
        let payload = json!({"content": "x".repeat(512)});

        let info = store
            .store_json("large-plan", "Large Plan", &payload)
            .expect("store artifact should succeed");

        assert!(info.is_file_backed);
        assert_eq!(info.reference, "artifact://large-plan");
        assert!(temp.path().join("artifacts/large-plan.json").exists());
        assert_eq!(
            store
                .retrieve_json("large-plan")
                .expect("retrieve should succeed"),
            payload
        );
    }

    #[test]
    fn remove_cleans_up_file_backed_payload() {
        let temp = TempDir::new().expect("temp dir should create");
        let store = ArtifactStore::new(Some(temp.path().to_path_buf()), 1)
            .expect("store should initialize");
        store
            .store_json("artifact-1", "Artifact", &json!({"content": "abc"}))
            .expect("store should succeed");

        let path = temp.path().join("artifacts/artifact-1.json");
        assert!(path.exists());

        store.remove("artifact-1").expect("remove should succeed");
        assert!(!path.exists());
        assert!(!store.has("artifact-1"));
    }

    #[test]
    fn reject_invalid_artifact_id() {
        let store = ArtifactStore::new(None, 1024).expect("store should initialize");
        let error = store
            .store_json("bad id", "Bad", &json!({"ok": true}))
            .expect_err("invalid id should fail");

        assert!(
            matches!(error, AttractorError::Runtime(message) if message.contains("unsupported characters"))
        );
    }
}
