use crate::AgentError;
use async_trait::async_trait;
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Instant;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::time::{Duration, sleep};

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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvVarPolicy {
    InheritAll,
    InheritNone,
    #[default]
    InheritCoreOnly,
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
    env_policy: EnvVarPolicy,
    default_command_timeout_ms: u64,
    max_command_timeout_ms: u64,
}

impl LocalExecutionEnvironment {
    pub fn new(working_directory: impl Into<PathBuf>) -> Self {
        Self {
            working_directory: working_directory.into(),
            platform: std::env::consts::OS.to_string(),
            os_version: detect_os_version(),
            env_policy: env_policy_from_env().unwrap_or_default(),
            default_command_timeout_ms: 10_000,
            max_command_timeout_ms: 600_000,
        }
    }

    pub fn with_env_policy(mut self, env_policy: EnvVarPolicy) -> Self {
        self.env_policy = env_policy;
        self
    }

    pub fn with_command_timeout_limits(
        mut self,
        default_timeout_ms: u64,
        max_timeout_ms: u64,
    ) -> Self {
        self.default_command_timeout_ms = default_timeout_ms.max(1);
        self.max_command_timeout_ms = max_timeout_ms.max(1);
        self
    }

    fn resolve_path(&self, path: &str) -> PathBuf {
        let path = Path::new(path);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.working_directory.join(path)
        }
    }

    fn effective_timeout_ms(&self, timeout_ms: u64) -> u64 {
        let requested = if timeout_ms == 0 {
            self.default_command_timeout_ms
        } else {
            timeout_ms
        };
        requested.min(self.max_command_timeout_ms)
    }

    fn build_command_env(
        &self,
        inherited_env: impl IntoIterator<Item = (String, String)>,
        env_overrides: Option<HashMap<String, String>>,
    ) -> HashMap<String, String> {
        let inherited: HashMap<String, String> = inherited_env.into_iter().collect();

        let mut env = match self.env_policy {
            EnvVarPolicy::InheritAll => inherited,
            EnvVarPolicy::InheritNone => HashMap::new(),
            EnvVarPolicy::InheritCoreOnly => {
                let mut core = HashMap::new();
                for key in core_env_keys() {
                    if let Some(value) = inherited.get(*key) {
                        core.insert((*key).to_string(), value.clone());
                    }
                }
                core
            }
        };

        if self.env_policy != EnvVarPolicy::InheritAll {
            env.retain(|key, _| !is_sensitive_env_var(key));
        }

        if let Some(overrides) = env_overrides {
            for (key, value) in overrides {
                env.insert(key, value);
            }
        }

        env
    }
}

#[async_trait]
impl ExecutionEnvironment for LocalExecutionEnvironment {
    async fn read_file(
        &self,
        path: &str,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> Result<String, AgentError> {
        let path = self.resolve_path(path);
        let content = tokio::fs::read_to_string(&path).await.map_err(|error| {
            AgentError::ExecutionEnvironment(format!(
                "failed to read '{}': {}",
                path.display(),
                error
            ))
        })?;

        if offset.is_none() && limit.is_none() {
            return Ok(content);
        }

        let start = offset.unwrap_or(1).saturating_sub(1);
        let max_lines = limit.unwrap_or(usize::MAX);
        let lines: Vec<&str> = content.lines().collect();
        if start >= lines.len() {
            return Ok(String::new());
        }

        let end = start.saturating_add(max_lines).min(lines.len());
        Ok(lines[start..end].join("\n"))
    }

    async fn write_file(&self, path: &str, content: &str) -> Result<(), AgentError> {
        let path = self.resolve_path(path);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                AgentError::ExecutionEnvironment(format!(
                    "failed to create directory '{}': {}",
                    parent.display(),
                    error
                ))
            })?;
        }
        tokio::fs::write(&path, content).await.map_err(|error| {
            AgentError::ExecutionEnvironment(format!(
                "failed to write '{}': {}",
                path.display(),
                error
            ))
        })
    }

    async fn file_exists(&self, path: &str) -> Result<bool, AgentError> {
        let path = self.resolve_path(path);
        Ok(tokio::fs::metadata(path).await.is_ok())
    }

    async fn list_directory(&self, path: &str, depth: usize) -> Result<Vec<DirEntry>, AgentError> {
        let root = self.resolve_path(path);
        let max_depth = depth.saturating_add(1);

        let mut entries = Vec::new();
        for entry in walkdir::WalkDir::new(&root)
            .min_depth(1)
            .max_depth(max_depth)
        {
            let entry = entry.map_err(|error| {
                AgentError::ExecutionEnvironment(format!(
                    "failed to list directory '{}': {}",
                    root.display(),
                    error
                ))
            })?;

            let relative = entry.path().strip_prefix(&root).unwrap_or(entry.path());
            let metadata = entry.metadata().map_err(|error| {
                AgentError::ExecutionEnvironment(format!(
                    "failed to read metadata for '{}': {}",
                    entry.path().display(),
                    error
                ))
            })?;

            entries.push(DirEntry {
                name: relative.to_string_lossy().to_string(),
                is_dir: metadata.is_dir(),
                size: if metadata.is_file() {
                    Some(metadata.len())
                } else {
                    None
                },
            });
        }

        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    async fn exec_command(
        &self,
        command: &str,
        timeout_ms: u64,
        working_dir: Option<&str>,
        env_vars: Option<HashMap<String, String>>,
    ) -> Result<ExecResult, AgentError> {
        let started = Instant::now();
        let timeout_ms = self.effective_timeout_ms(timeout_ms);
        let working_dir = working_dir
            .map(|path| self.resolve_path(path))
            .unwrap_or_else(|| self.working_directory.clone());

        let mut cmd = build_shell_command(command);
        cmd.current_dir(working_dir);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        #[cfg(unix)]
        {
            cmd.process_group(0);
        }

        let env = self.build_command_env(std::env::vars(), env_vars);
        cmd.env_clear();
        cmd.envs(env);

        let mut child = cmd.spawn().map_err(|error| {
            AgentError::ExecutionEnvironment(format!(
                "failed to spawn command '{}': {}",
                command, error
            ))
        })?;

        let stdout_task = tokio::spawn(read_pipe(child.stdout.take()));
        let stderr_task = tokio::spawn(read_pipe(child.stderr.take()));

        let mut timed_out = false;
        let status =
            match tokio::time::timeout(Duration::from_millis(timeout_ms), child.wait()).await {
                Ok(wait_result) => wait_result.map_err(|error| {
                    AgentError::ExecutionEnvironment(format!(
                        "failed to wait for command '{}': {}",
                        command, error
                    ))
                })?,
                Err(_) => {
                    timed_out = true;
                    terminate_command(&mut child).await?;
                    child.wait().await.map_err(|error| {
                        AgentError::ExecutionEnvironment(format!(
                            "failed to collect timed-out command '{}': {}",
                            command, error
                        ))
                    })?
                }
            };

        let mut stdout = String::from_utf8_lossy(&stdout_task.await.map_err(|error| {
            AgentError::ExecutionEnvironment(format!(
                "stdout reader task failed for '{}': {}",
                command, error
            ))
        })?)
        .to_string();
        let mut stderr = String::from_utf8_lossy(&stderr_task.await.map_err(|error| {
            AgentError::ExecutionEnvironment(format!(
                "stderr reader task failed for '{}': {}",
                command, error
            ))
        })?)
        .to_string();

        if timed_out {
            if !stdout.is_empty() && !stdout.ends_with('\n') {
                stdout.push('\n');
            }
            if !stderr.is_empty() && !stderr.ends_with('\n') {
                stderr.push('\n');
            }
            stderr.push_str(&format!(
                "[ERROR: Command timed out after {}ms. Partial output is shown above.\nYou can retry with a longer timeout by setting the timeout_ms parameter.]",
                timeout_ms
            ));
        }

        Ok(ExecResult {
            stdout,
            stderr,
            exit_code: status.code().unwrap_or(if timed_out { 124 } else { -1 }),
            timed_out,
            duration_ms: started.elapsed().as_millis(),
        })
    }

    async fn grep(
        &self,
        pattern: &str,
        path: &str,
        options: GrepOptions,
    ) -> Result<String, AgentError> {
        let path = self.resolve_path(path);
        if ripgrep_available() {
            match run_ripgrep(pattern, &path, &options).await {
                Ok(output) => return Ok(output),
                Err(_) => {
                    // Fallback handled below.
                }
            }
        }

        grep_fallback(pattern, &path, &options).await
    }

    async fn glob(&self, pattern: &str, path: &str) -> Result<Vec<String>, AgentError> {
        let root = self.resolve_path(path);
        let pattern_path = if Path::new(pattern).is_absolute() {
            PathBuf::from(pattern)
        } else {
            root.join(pattern)
        };
        let pattern_string = pattern_path.to_string_lossy().to_string();

        let mut matches = Vec::new();
        for entry in glob::glob(&pattern_string).map_err(|error| {
            AgentError::ExecutionEnvironment(format!(
                "invalid glob pattern '{}': {}",
                pattern, error
            ))
        })? {
            let entry = entry.map_err(|error| {
                AgentError::ExecutionEnvironment(format!(
                    "glob match failed for '{}': {}",
                    pattern_string, error
                ))
            })?;
            matches.push(entry);
        }

        let mut by_mtime: Vec<(PathBuf, std::time::SystemTime)> = matches
            .into_iter()
            .map(|path| {
                let modified = std::fs::metadata(&path)
                    .and_then(|meta| meta.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                (path, modified)
            })
            .collect();

        by_mtime.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(by_mtime
            .into_iter()
            .map(|(path, _)| path.to_string_lossy().to_string())
            .collect())
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

fn build_shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd.exe");
        cmd.arg("/c").arg(command);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("/bin/bash");
        cmd.arg("-c").arg(command);
        cmd
    }
}

async fn read_pipe<R>(pipe: Option<R>) -> Vec<u8>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    match pipe {
        Some(mut reader) => {
            let mut bytes = Vec::new();
            let _ = reader.read_to_end(&mut bytes).await;
            bytes
        }
        None => Vec::new(),
    }
}

#[cfg(unix)]
async fn terminate_command(child: &mut Child) -> Result<(), AgentError> {
    use nix::sys::signal::{Signal, killpg};
    use nix::unistd::Pid;

    if let Some(pid) = child.id() {
        let _ = killpg(Pid::from_raw(pid as i32), Signal::SIGTERM);
    }

    sleep(Duration::from_secs(2)).await;
    if child
        .try_wait()
        .map_err(|error| {
            AgentError::ExecutionEnvironment(format!("failed checking child status: {}", error))
        })?
        .is_none()
    {
        if let Some(pid) = child.id() {
            let _ = killpg(Pid::from_raw(pid as i32), Signal::SIGKILL);
        }
    }
    Ok(())
}

#[cfg(not(unix))]
async fn terminate_command(child: &mut Child) -> Result<(), AgentError> {
    child.kill().await.map_err(|error| {
        AgentError::ExecutionEnvironment(format!("failed to terminate child process: {}", error))
    })
}

async fn run_ripgrep(
    pattern: &str,
    path: &Path,
    options: &GrepOptions,
) -> Result<String, AgentError> {
    let mut cmd = Command::new("rg");
    cmd.arg("--line-number")
        .arg("--no-heading")
        .arg("--color")
        .arg("never");
    if options.case_insensitive {
        cmd.arg("--ignore-case");
    }
    if let Some(glob_filter) = &options.glob_filter {
        cmd.arg("--glob").arg(glob_filter);
    }
    if let Some(max) = options.max_results {
        cmd.arg("--max-count").arg(max.to_string());
    }
    cmd.arg(pattern).arg(path);

    let output = cmd.output().await.map_err(|error| {
        AgentError::ExecutionEnvironment(format!("failed to execute ripgrep: {}", error))
    })?;

    let exit = output.status.code().unwrap_or(-1);
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }
    if exit == 1 {
        return Ok(String::new());
    }

    Err(AgentError::ExecutionEnvironment(format!(
        "ripgrep failed with exit code {}: {}",
        exit,
        String::from_utf8_lossy(&output.stderr)
    )))
}

async fn grep_fallback(
    pattern: &str,
    path: &Path,
    options: &GrepOptions,
) -> Result<String, AgentError> {
    let regex = RegexBuilder::new(pattern)
        .case_insensitive(options.case_insensitive)
        .build()
        .map_err(|error| {
            AgentError::ExecutionEnvironment(format!("invalid regex '{}': {}", pattern, error))
        })?;

    let glob_filter = options
        .glob_filter
        .as_ref()
        .map(|pattern| glob::Pattern::new(pattern))
        .transpose()
        .map_err(|error| {
            AgentError::ExecutionEnvironment(format!(
                "invalid glob filter in grep options: {}",
                error
            ))
        })?;

    let mut matches = Vec::new();
    let max_results = options.max_results.unwrap_or(100);
    let files = enumerate_files(path)?;

    for file in files {
        if let Some(filter) = &glob_filter {
            if !filter.matches_path(&file) {
                continue;
            }
        }

        let content = match tokio::fs::read_to_string(&file).await {
            Ok(content) => content,
            Err(_) => continue,
        };

        for (idx, line) in content.lines().enumerate() {
            if regex.is_match(line) {
                matches.push(format!("{}:{}:{}", file.display(), idx + 1, line));
                if matches.len() >= max_results {
                    return Ok(matches.join("\n"));
                }
            }
        }
    }

    Ok(matches.join("\n"))
}

fn enumerate_files(path: &Path) -> Result<Vec<PathBuf>, AgentError> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }

    if !path.exists() {
        return Err(AgentError::ExecutionEnvironment(format!(
            "path not found for grep: {}",
            path.display()
        )));
    }

    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(path) {
        let entry = entry.map_err(|error| {
            AgentError::ExecutionEnvironment(format!(
                "failed to walk path '{}' for grep: {}",
                path.display(),
                error
            ))
        })?;
        if entry.file_type().is_file() {
            files.push(entry.path().to_path_buf());
        }
    }
    Ok(files)
}

fn ripgrep_available() -> bool {
    static HAS_RG: OnceLock<bool> = OnceLock::new();
    *HAS_RG.get_or_init(|| {
        std::process::Command::new("rg")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    })
}

fn detect_os_version() -> String {
    #[cfg(unix)]
    {
        if let Ok(output) = std::process::Command::new("uname").arg("-r").output() {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !text.is_empty() {
                    return text;
                }
            }
        }
    }
    "unknown".to_string()
}

fn env_policy_from_env() -> Option<EnvVarPolicy> {
    let raw = std::env::var("FORGE_AGENT_ENV_POLICY").ok()?;
    match raw.trim().to_lowercase().as_str() {
        "all" | "inherit_all" => Some(EnvVarPolicy::InheritAll),
        "none" | "inherit_none" => Some(EnvVarPolicy::InheritNone),
        "core" | "core_only" | "inherit_core_only" => Some(EnvVarPolicy::InheritCoreOnly),
        _ => None,
    }
}

fn core_env_keys() -> &'static [&'static str] {
    &[
        "PATH",
        "HOME",
        "USER",
        "SHELL",
        "LANG",
        "TERM",
        "TMPDIR",
        "TMP",
        "TEMP",
        "GOPATH",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "NVM_DIR",
        "NPM_CONFIG_PREFIX",
        "PNPM_HOME",
        "PYENV_ROOT",
        "VIRTUAL_ENV",
    ]
}

fn is_sensitive_env_var(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    key.ends_with("_API_KEY")
        || key.ends_with("_SECRET")
        || key.ends_with("_TOKEN")
        || key.ends_with("_PASSWORD")
        || key.ends_with("_CREDENTIAL")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    #[tokio::test(flavor = "current_thread")]
    async fn read_write_and_file_exists_work_for_local_environment() {
        let dir = tempdir().expect("temp dir should be created");
        let env = LocalExecutionEnvironment::new(dir.path());
        env.write_file("nested/file.txt", "a\nb\nc")
            .await
            .expect("write should succeed");

        let content = env
            .read_file("nested/file.txt", Some(2), Some(1))
            .await
            .expect("read should succeed");
        assert_eq!(content, "b");
        assert!(
            env.file_exists("nested/file.txt")
                .await
                .expect("exists should succeed")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_directory_respects_depth() {
        let dir = tempdir().expect("temp dir should be created");
        let env = LocalExecutionEnvironment::new(dir.path());
        env.write_file("a.txt", "root").await.expect("write root");
        env.write_file("nested/b.txt", "nested")
            .await
            .expect("write nested");
        env.write_file("nested/deeper/c.txt", "deep")
            .await
            .expect("write deep");

        let entries = env
            .list_directory(".", 1)
            .await
            .expect("list should succeed");
        let names: Vec<String> = entries.into_iter().map(|entry| entry.name).collect();
        assert!(names.contains(&"a.txt".to_string()));
        assert!(names.contains(&"nested".to_string()));
        assert!(names.contains(&"nested/b.txt".to_string()));
        assert!(!names.contains(&"nested/deeper/c.txt".to_string()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn grep_and_glob_find_expected_files() {
        let dir = tempdir().expect("temp dir should be created");
        let env = LocalExecutionEnvironment::new(dir.path());
        env.write_file("src/lib.rs", "fn alpha() {}\nfn beta() {}")
            .await
            .expect("write lib");
        env.write_file("src/main.rs", "fn main() { alpha(); }")
            .await
            .expect("write main");

        let grep_output = env
            .grep(
                "alpha",
                ".",
                GrepOptions {
                    glob_filter: Some("*.rs".to_string()),
                    case_insensitive: false,
                    max_results: Some(10),
                },
            )
            .await
            .expect("grep should succeed");
        assert!(grep_output.contains("alpha"));

        let globbed = env.glob("**/*.rs", ".").await.expect("glob should succeed");
        assert!(globbed.iter().any(|path| path.ends_with("src/lib.rs")));
        assert!(globbed.iter().any(|path| path.ends_with("src/main.rs")));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn exec_command_timeout_returns_partial_output_and_error_message() {
        let dir = tempdir().expect("temp dir should be created");
        let env =
            LocalExecutionEnvironment::new(dir.path()).with_command_timeout_limits(10_000, 150);

        let result = env
            .exec_command("echo begin; sleep 2; echo end", 5_000, None, None)
            .await
            .expect("command should return a timeout result");

        assert!(result.timed_out);
        assert!(result.stdout.contains("begin"));
        assert!(result.stderr.contains("Command timed out after 150ms"));
    }

    #[test]
    fn env_filtering_excludes_sensitive_vars_by_default_and_allows_core() {
        let env =
            LocalExecutionEnvironment::new(".").with_env_policy(EnvVarPolicy::InheritCoreOnly);
        let filtered = env.build_command_env(
            BTreeMap::from([
                ("PATH".to_string(), "/bin".to_string()),
                ("HOME".to_string(), "/home/user".to_string()),
                ("SERVICE_API_KEY".to_string(), "secret".to_string()),
                ("RANDOM_VAR".to_string(), "value".to_string()),
            ]),
            None,
        );

        assert_eq!(filtered.get("PATH"), Some(&"/bin".to_string()));
        assert_eq!(filtered.get("HOME"), Some(&"/home/user".to_string()));
        assert!(!filtered.contains_key("SERVICE_API_KEY"));
        assert!(!filtered.contains_key("RANDOM_VAR"));
    }

    #[test]
    fn env_filtering_inherit_all_keeps_sensitive_vars() {
        let env = LocalExecutionEnvironment::new(".").with_env_policy(EnvVarPolicy::InheritAll);
        let filtered = env.build_command_env(
            BTreeMap::from([("SERVICE_API_KEY".to_string(), "secret".to_string())]),
            None,
        );
        assert_eq!(filtered.get("SERVICE_API_KEY"), Some(&"secret".to_string()));
    }

    #[test]
    fn timeout_value_zero_uses_default_and_clamps_to_max() {
        let env = LocalExecutionEnvironment::new(".").with_command_timeout_limits(10_000, 600_000);
        assert_eq!(env.effective_timeout_ms(0), 10_000);
        assert_eq!(env.effective_timeout_ms(700_000), 600_000);
    }
}
