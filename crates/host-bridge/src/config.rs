use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "host-bridge",
    version,
    about = "Lightspeed guest OS host bridge"
)]
pub struct BridgeArgs {
    #[arg(long, env = "LIGHTSPEED_GATEWAY_URL")]
    pub gateway_url: String,

    #[arg(long, env = "LIGHTSPEED_HOST_BRIDGE_PROVIDER_ID")]
    pub provider_id: Option<String>,

    #[arg(long, env = "LIGHTSPEED_PROVIDER_TOKEN")]
    pub provider_token: Option<String>,

    #[arg(
        long,
        env = "LIGHTSPEED_HOST_BRIDGE_TARGET_ID",
        default_value = "local"
    )]
    pub target_id: String,

    #[arg(
        long,
        env = "LIGHTSPEED_HOST_BRIDGE_LISTEN",
        default_value = "127.0.0.1:0"
    )]
    pub listen: SocketAddr,

    #[arg(long, env = "LIGHTSPEED_HOST_BRIDGE_ADVERTISE_URL")]
    pub advertise_url: Option<String>,

    #[arg(long, env = "LIGHTSPEED_HOST_BRIDGE_CWD")]
    pub cwd: Option<PathBuf>,

    #[arg(long, env = "LIGHTSPEED_HOST_BRIDGE_FS_ROOT")]
    pub fs_root: Option<PathBuf>,

    #[arg(long, env = "LIGHTSPEED_HOST_BRIDGE_STATE_DIR")]
    pub state_dir: Option<PathBuf>,

    /// Extra target metadata advertised to the environment registry, as
    /// `key=value` entries. Repeat the flag or comma-separate values.
    #[arg(
        long = "metadata",
        env = "LIGHTSPEED_HOST_BRIDGE_METADATA",
        value_delimiter = ','
    )]
    pub metadata: Vec<String>,

    #[arg(long, default_value_t = 10_000)]
    pub heartbeat_interval_ms: u64,

    #[arg(long, default_value_t = 30_000)]
    pub lease_ttl_ms: u64,

    #[arg(long, default_value_t = false)]
    pub read_only_fs: bool,
}

#[derive(Clone, Debug)]
pub struct BridgeConfig {
    pub gateway_url: String,
    pub provider_id: String,
    pub provider_token: Option<String>,
    pub target_id: String,
    pub listen: SocketAddr,
    pub advertise_url: Option<String>,
    pub cwd: PathBuf,
    pub fs_root: PathBuf,
    pub state_dir: PathBuf,
    pub metadata: BTreeMap<String, String>,
    pub heartbeat_interval: Duration,
    pub lease_ttl: Duration,
    pub read_only_fs: bool,
}

impl BridgeArgs {
    pub fn into_config(self) -> Result<BridgeConfig> {
        if self.gateway_url.trim().is_empty() {
            bail!("--gateway-url must not be empty");
        }
        if self.target_id.trim().is_empty() {
            bail!("--target-id must not be empty");
        }
        if self.lease_ttl_ms == 0 {
            bail!("--lease-ttl-ms must be greater than zero");
        }
        if self.heartbeat_interval_ms == 0 {
            bail!("--heartbeat-interval-ms must be greater than zero");
        }

        let cwd = match self.cwd {
            Some(cwd) => cwd,
            None => std::env::current_dir().context("read current directory")?,
        };
        let cwd = canonical_dir(cwd, "cwd")?;
        let fs_root = canonical_dir(
            self.fs_root.unwrap_or_else(|| native_filesystem_root(&cwd)),
            "fs root",
        )?;
        if !cwd.starts_with(&fs_root) {
            bail!(
                "cwd must be inside fs root: cwd={}, fs_root={}",
                cwd.display(),
                fs_root.display()
            );
        }
        let state_dir = match self.state_dir {
            Some(state_dir) if state_dir.as_os_str().is_empty() => {
                bail!("--state-dir must not be empty");
            }
            Some(state_dir) if state_dir.is_absolute() => state_dir,
            Some(state_dir) => cwd.join(state_dir),
            None => cwd.join(".lightspeed"),
        };

        Ok(BridgeConfig {
            gateway_url: self.gateway_url,
            provider_id: self.provider_id.unwrap_or_else(ephemeral_provider_id),
            provider_token: self.provider_token,
            target_id: self.target_id,
            listen: self.listen,
            advertise_url: self.advertise_url,
            cwd,
            fs_root,
            state_dir,
            metadata: parse_metadata(&self.metadata)?,
            heartbeat_interval: Duration::from_millis(self.heartbeat_interval_ms),
            lease_ttl: Duration::from_millis(self.lease_ttl_ms),
            read_only_fs: self.read_only_fs,
        })
    }
}

/// Keys the bridge itself stamps on the advertised target; configured
/// metadata must not shadow them.
const RESERVED_METADATA_KEYS: &[&str] = &["kind", "fsRoot"];

fn parse_metadata(entries: &[String]) -> Result<BTreeMap<String, String>> {
    let mut metadata = BTreeMap::new();
    for entry in entries {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let Some((key, value)) = entry.split_once('=') else {
            bail!("--metadata entries must be key=value, got: {entry}");
        };
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() || value.is_empty() {
            bail!("--metadata keys and values must not be empty, got: {entry}");
        }
        if RESERVED_METADATA_KEYS.contains(&key) {
            bail!("--metadata key is reserved by the bridge: {key}");
        }
        if metadata.insert(key.to_owned(), value.to_owned()).is_some() {
            bail!("--metadata key given more than once: {key}");
        }
    }
    Ok(metadata)
}

fn native_filesystem_root(path: &Path) -> PathBuf {
    path.ancestors()
        .last()
        .expect("an absolute canonical path has a filesystem root")
        .to_path_buf()
}

impl BridgeConfig {
    pub fn advertise_base_url(&self, local_addr: SocketAddr) -> String {
        self.advertise_url
            .clone()
            .unwrap_or_else(|| format!("ws://{local_addr}"))
            .trim_end_matches('/')
            .to_owned()
    }

    pub fn lease_ttl_ms_i64(&self) -> i64 {
        self.lease_ttl.as_millis().min(i64::MAX as u128) as i64
    }

    pub fn display_name(&self) -> String {
        format!("host bridge {}", self.provider_id)
    }
}

fn canonical_dir(path: PathBuf, label: &str) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("canonicalize {label}: {}", path.display()))?;
    if !canonical.is_dir() {
        bail!("{label} must be a directory: {}", canonical.display());
    }
    Ok(canonical)
}

fn ephemeral_provider_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("host-bridge-{}-{millis}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(cwd: PathBuf) -> BridgeArgs {
        BridgeArgs {
            gateway_url: "http://127.0.0.1:18080/rpc".to_owned(),
            provider_id: Some("test-provider".to_owned()),
            provider_token: None,
            target_id: "local".to_owned(),
            listen: "127.0.0.1:0".parse().expect("listen address"),
            advertise_url: None,
            cwd: Some(cwd),
            fs_root: None,
            state_dir: None,
            metadata: Vec::new(),
            heartbeat_interval_ms: 10_000,
            lease_ttl_ms: 30_000,
            read_only_fs: false,
        }
    }

    #[test]
    fn defaults_fs_root_to_native_root_and_state_dir_to_cwd() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cwd = temp.path().canonicalize().expect("canonical cwd");

        let config = args(cwd.clone()).into_config().expect("config");

        assert_eq!(config.fs_root, native_filesystem_root(&cwd));
        assert_eq!(config.state_dir, cwd.join(".lightspeed"));
    }

    #[test]
    fn resolves_relative_state_dir_from_cwd() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cwd = temp.path().canonicalize().expect("canonical cwd");
        let mut args = args(cwd.clone());
        args.state_dir = Some(PathBuf::from("bridge-state"));

        let config = args.into_config().expect("config");

        assert_eq!(config.state_dir, cwd.join("bridge-state"));
    }

    #[test]
    fn parses_metadata_entries() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cwd = temp.path().canonicalize().expect("canonical cwd");
        let mut args = args(cwd);
        args.metadata = vec![
            "lsbot.role=pack-dev".to_owned(),
            " lsbot.pack = hello ".to_owned(),
        ];

        let config = args.into_config().expect("config");

        assert_eq!(
            config.metadata,
            BTreeMap::from([
                ("lsbot.role".to_owned(), "pack-dev".to_owned()),
                ("lsbot.pack".to_owned(), "hello".to_owned()),
            ])
        );
    }

    #[test]
    fn rejects_malformed_reserved_and_duplicate_metadata() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cwd = temp.path().canonicalize().expect("canonical cwd");

        for (entries, message) in [
            (vec!["no-equals".to_owned()], "must be key=value"),
            (vec!["=value".to_owned()], "must not be empty"),
            (vec!["key=".to_owned()], "must not be empty"),
            (vec!["kind=other".to_owned()], "reserved"),
            (vec!["fsRoot=/tmp".to_owned()], "reserved"),
            (vec!["a=1".to_owned(), "a=2".to_owned()], "more than once"),
        ] {
            let mut args = args(cwd.clone());
            args.metadata = entries;
            let error = args.into_config().expect_err("invalid metadata");
            assert!(
                error.to_string().contains(message),
                "expected {message:?} in {error}"
            );
        }
    }
}
