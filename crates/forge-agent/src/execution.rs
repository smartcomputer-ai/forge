use crate::AgentError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrepOptions {
    pub glob_filter: Option<String>,
    pub case_insensitive: bool,
    pub max_results: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub timed_out: bool,
    pub duration_ms: u128,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: Option<u64>,
}

#[async_trait]
pub trait ExecutionEnvironment: Send + Sync {
    async fn read_file(
        &self,
        path: &str,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> Result<String, AgentError>;

    async fn write_file(&self, path: &str, content: &str) -> Result<(), AgentError>;
    async fn file_exists(&self, path: &str) -> Result<bool, AgentError>;
    async fn list_directory(&self, path: &str, depth: usize) -> Result<Vec<DirEntry>, AgentError>;

    async fn exec_command(
        &self,
        command: &str,
        timeout_ms: u64,
        working_dir: Option<&str>,
        env_vars: Option<HashMap<String, String>>,
    ) -> Result<ExecResult, AgentError>;

    async fn grep(
        &self,
        pattern: &str,
        path: &str,
        options: GrepOptions,
    ) -> Result<String, AgentError>;

    async fn glob(&self, pattern: &str, path: &str) -> Result<Vec<String>, AgentError>;

    async fn initialize(&self) -> Result<(), AgentError> {
        Ok(())
    }

    async fn cleanup(&self) -> Result<(), AgentError> {
        Ok(())
    }

    fn working_directory(&self) -> &Path;
    fn platform(&self) -> &str;
    fn os_version(&self) -> &str;
}

#[derive(Clone, Debug)]
pub struct LocalExecutionEnvironment {
    working_directory: PathBuf,
    platform: String,
    os_version: String,
}

impl LocalExecutionEnvironment {
    pub fn new(working_directory: impl Into<PathBuf>) -> Self {
        Self {
            working_directory: working_directory.into(),
            platform: std::env::consts::OS.to_string(),
            os_version: "unknown".to_string(),
        }
    }
}

#[async_trait]
impl ExecutionEnvironment for LocalExecutionEnvironment {
    async fn read_file(
        &self,
        _path: &str,
        _offset: Option<usize>,
        _limit: Option<usize>,
    ) -> Result<String, AgentError> {
        Err(AgentError::NotImplemented(
            "LocalExecutionEnvironment::read_file".to_string(),
        ))
    }

    async fn write_file(&self, _path: &str, _content: &str) -> Result<(), AgentError> {
        Err(AgentError::NotImplemented(
            "LocalExecutionEnvironment::write_file".to_string(),
        ))
    }

    async fn file_exists(&self, _path: &str) -> Result<bool, AgentError> {
        Err(AgentError::NotImplemented(
            "LocalExecutionEnvironment::file_exists".to_string(),
        ))
    }

    async fn list_directory(
        &self,
        _path: &str,
        _depth: usize,
    ) -> Result<Vec<DirEntry>, AgentError> {
        Err(AgentError::NotImplemented(
            "LocalExecutionEnvironment::list_directory".to_string(),
        ))
    }

    async fn exec_command(
        &self,
        _command: &str,
        _timeout_ms: u64,
        _working_dir: Option<&str>,
        _env_vars: Option<HashMap<String, String>>,
    ) -> Result<ExecResult, AgentError> {
        Err(AgentError::NotImplemented(
            "LocalExecutionEnvironment::exec_command".to_string(),
        ))
    }

    async fn grep(
        &self,
        _pattern: &str,
        _path: &str,
        _options: GrepOptions,
    ) -> Result<String, AgentError> {
        Err(AgentError::NotImplemented(
            "LocalExecutionEnvironment::grep".to_string(),
        ))
    }

    async fn glob(&self, _pattern: &str, _path: &str) -> Result<Vec<String>, AgentError> {
        Err(AgentError::NotImplemented(
            "LocalExecutionEnvironment::glob".to_string(),
        ))
    }

    fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    fn platform(&self) -> &str {
        &self.platform
    }

    fn os_version(&self) -> &str {
        &self.os_version
    }
}
