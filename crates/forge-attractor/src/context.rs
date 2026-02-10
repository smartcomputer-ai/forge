use crate::AttractorError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

pub type RuntimeContext = BTreeMap<String, Value>;

const MAX_KEY_LENGTH: usize = 256;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ContextSnapshot {
    pub values: RuntimeContext,
    pub logs: Vec<String>,
}

#[derive(Clone, Default)]
pub struct ContextStore {
    inner: Arc<RwLock<ContextState>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct ContextState {
    values: RuntimeContext,
    logs: Vec<String>,
}

impl ContextStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_values(values: RuntimeContext) -> Self {
        Self {
            inner: Arc::new(RwLock::new(ContextState {
                values,
                logs: Vec::new(),
            })),
        }
    }

    pub fn set(&self, key: impl Into<String>, value: Value) -> Result<(), AttractorError> {
        let key = key.into();
        validate_context_key(&key)?;
        let mut state = self
            .inner
            .write()
            .map_err(|_| AttractorError::Runtime("context write lock poisoned".to_string()))?;
        state.values.insert(key, value);
        Ok(())
    }

    pub fn get(&self, key: &str) -> Result<Option<Value>, AttractorError> {
        let state = self
            .inner
            .read()
            .map_err(|_| AttractorError::Runtime("context read lock poisoned".to_string()))?;
        Ok(state.values.get(key).cloned())
    }

    pub fn apply_updates(&self, updates: &RuntimeContext) -> Result<(), AttractorError> {
        if updates.is_empty() {
            return Ok(());
        }

        let mut state = self
            .inner
            .write()
            .map_err(|_| AttractorError::Runtime("context write lock poisoned".to_string()))?;
        for (key, value) in updates {
            validate_context_key(key)?;
            state.values.insert(key.clone(), value.clone());
        }
        Ok(())
    }

    pub fn remove(&self, key: &str) -> Result<(), AttractorError> {
        let mut state = self
            .inner
            .write()
            .map_err(|_| AttractorError::Runtime("context write lock poisoned".to_string()))?;
        state.values.remove(key);
        Ok(())
    }

    pub fn append_log(&self, entry: impl Into<String>) -> Result<(), AttractorError> {
        let mut state = self
            .inner
            .write()
            .map_err(|_| AttractorError::Runtime("context write lock poisoned".to_string()))?;
        state.logs.push(entry.into());
        Ok(())
    }

    pub fn snapshot(&self) -> Result<ContextSnapshot, AttractorError> {
        let state = self
            .inner
            .read()
            .map_err(|_| AttractorError::Runtime("context read lock poisoned".to_string()))?;
        Ok(ContextSnapshot {
            values: state.values.clone(),
            logs: state.logs.clone(),
        })
    }

    pub fn clone_isolated(&self) -> Result<Self, AttractorError> {
        let snapshot = self.snapshot()?;
        Ok(Self {
            inner: Arc::new(RwLock::new(ContextState {
                values: snapshot.values,
                logs: snapshot.logs,
            })),
        })
    }
}

pub fn validate_context_key(key: &str) -> Result<(), AttractorError> {
    if key.is_empty() {
        return Err(AttractorError::Runtime(
            "context key cannot be empty".to_string(),
        ));
    }
    if key.len() > MAX_KEY_LENGTH {
        return Err(AttractorError::Runtime(format!(
            "context key '{}' exceeds max length {}",
            key, MAX_KEY_LENGTH
        )));
    }

    for segment in key.split('.') {
        if segment.is_empty() {
            return Err(AttractorError::Runtime(format!(
                "context key '{}' contains an empty namespace segment",
                key
            )));
        }
        validate_key_segment(segment, key)?;
    }

    Ok(())
}

fn validate_key_segment(segment: &str, full_key: &str) -> Result<(), AttractorError> {
    let mut chars = segment.chars();
    let Some(first) = chars.next() else {
        return Err(AttractorError::Runtime(format!(
            "context key '{}' contains an empty namespace segment",
            full_key
        )));
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err(AttractorError::Runtime(format!(
            "context key '{}' has invalid segment '{}'",
            full_key, segment
        )));
    }

    if chars.any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')) {
        return Err(AttractorError::Runtime(format!(
            "context key '{}' has invalid segment '{}'",
            full_key, segment
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn set_and_snapshot_round_trip() {
        let store = ContextStore::new();

        store
            .set("graph.goal", Value::String("ship".to_string()))
            .expect("set graph goal should succeed");
        store
            .set("context.plan.status", Value::String("done".to_string()))
            .expect("set context key should succeed");
        store
            .append_log("stage plan completed")
            .expect("append log should succeed");

        let snapshot = store.snapshot().expect("snapshot should succeed");
        assert_eq!(
            snapshot.values.get("graph.goal"),
            Some(&Value::String("ship".to_string()))
        );
        assert_eq!(
            snapshot.values.get("context.plan.status"),
            Some(&Value::String("done".to_string()))
        );
        assert_eq!(snapshot.logs, vec!["stage plan completed".to_string()]);
    }

    #[test]
    fn apply_updates_merges_values() {
        let store = ContextStore::from_values(BTreeMap::from([(
            "context.existing".to_string(),
            Value::String("yes".to_string()),
        )]));

        store
            .apply_updates(&BTreeMap::from([
                ("context.new".to_string(), json!(123)),
                ("outcome".to_string(), Value::String("success".to_string())),
            ]))
            .expect("apply updates should succeed");

        let snapshot = store.snapshot().expect("snapshot should succeed");
        assert_eq!(snapshot.values.get("context.existing"), Some(&json!("yes")));
        assert_eq!(snapshot.values.get("context.new"), Some(&json!(123)));
        assert_eq!(snapshot.values.get("outcome"), Some(&json!("success")));
    }

    #[test]
    fn clone_isolated_creates_independent_copy() {
        let original = ContextStore::new();
        original
            .set("context.key", Value::String("original".to_string()))
            .expect("set should succeed");

        let cloned = original
            .clone_isolated()
            .expect("clone isolated should succeed");
        cloned
            .set("context.key", Value::String("clone".to_string()))
            .expect("set on clone should succeed");

        assert_eq!(
            original.get("context.key").expect("get should succeed"),
            Some(json!("original"))
        );
        assert_eq!(
            cloned.get("context.key").expect("get should succeed"),
            Some(json!("clone"))
        );
    }

    #[test]
    fn reject_invalid_context_keys() {
        let store = ContextStore::new();
        let error = store
            .set("context.bad key", Value::String("x".to_string()))
            .expect_err("invalid key should fail");
        assert!(
            matches!(error, AttractorError::Runtime(message) if message.contains("invalid segment"))
        );
    }
}
