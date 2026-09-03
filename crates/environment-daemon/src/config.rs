use std::{
    collections::BTreeMap,
    ffi::OsString,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use environment_protocol::{registration::validate_registration_metadata, shared::SecretString};

/// Listener address when the daemon is not registering outbound.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:19091";

/// Every daemon configuration variable shares this prefix; all of them are
/// removed from the environment of every child process and job.
pub const ENV_PREFIX: &str = "LIGHTSPEED_ENVD_";
/// Internal one-exec handoff for a registration key that configuration has
/// already read and scrubbed. Never documented as operator configuration.
pub(crate) const REEXEC_REGISTRATION_KEY_ENV: &str = "LIGHTSPEED_ENVD_REEXEC_REGISTRATION_KEY";

#[derive(Parser, Debug)]
#[command(
    name = "lightspeed-envd",
    version = release_info::LONG_VERSION,
    about = "Lightspeed environment execution daemon"
)]
pub struct DaemonArgs {
    #[command(subcommand)]
    pub command: Option<DaemonCommand>,

    /// WebSocket listener reachable by Lightspeed or an environment provider.
    /// Defaults to 127.0.0.1:19091 unless a gateway URL makes the daemon
    /// dial out instead.
    #[arg(long, env = "LIGHTSPEED_ENVD_LISTEN")]
    pub listen: Option<SocketAddr>,

    /// Public environment-gateway connect URL to dial outbound
    /// (`wss://host/environment-gateway/connect`).
    #[arg(long, env = "LIGHTSPEED_ENVD_GATEWAY_URL", global = true)]
    pub gateway_url: Option<String>,

    /// Discovery document used for manual or automatic upgrades. Defaults to
    /// `/.well-known/lightspeed-envd` on the configured gateway host.
    #[arg(long, env = "LIGHTSPEED_ENVD_DISCOVERY_URL", global = true)]
    pub discovery_url: Option<String>,

    /// Upgrade automatically when an outbound gateway advertises a different
    /// environment protocol version.
    #[arg(long, env = "LIGHTSPEED_ENVD_AUTO_UPGRADE", default_value_t = false)]
    pub auto_upgrade: bool,

    /// Registration key admitting this daemon as a new environment. Read
    /// once and removed from the process environment.
    #[arg(long, env = "LIGHTSPEED_ENVD_REGISTRATION_KEY", hide_env_values = true)]
    pub registration_key: Option<String>,

    /// File holding the registration key; the safer form for sandboxes.
    #[arg(long, env = "LIGHTSPEED_ENVD_REGISTRATION_KEY_FILE")]
    pub registration_key_file: Option<PathBuf>,

    /// Display-name hint recorded on the environment at first registration.
    #[arg(long, env = "LIGHTSPEED_ENVD_REGISTRATION_NAME")]
    pub registration_name: Option<String>,

    /// Bounded JSON object of string correlation metadata.
    #[arg(long, env = "LIGHTSPEED_ENVD_REGISTRATION_METADATA")]
    pub registration_metadata: Option<String>,

    /// Where to write the registration receipt JSON once accepted.
    #[arg(long, env = "LIGHTSPEED_ENVD_REGISTRATION_RECEIPT")]
    pub registration_receipt: Option<PathBuf>,

    /// PEM file with additional TLS trust anchors for the gateway.
    #[arg(long, env = "LIGHTSPEED_ENVD_CA_FILE", global = true)]
    pub ca_file: Option<PathBuf>,

    #[arg(long, env = "LIGHTSPEED_ENVD_CWD")]
    pub cwd: Option<PathBuf>,

    #[arg(long, env = "LIGHTSPEED_ENVD_FS_ROOT")]
    pub fs_root: Option<PathBuf>,

    #[arg(long, env = "LIGHTSPEED_ENVD_STATE_DIR")]
    pub state_dir: Option<PathBuf>,

    #[arg(long, default_value_t = false)]
    pub read_only_fs: bool,

    /// Print this binary's build facts as one JSON object and exit: name,
    /// version, git sha, target, and the environment protocol version it
    /// speaks. For orchestrators checking a downloaded daemon against a
    /// deployment's discovery document.
    #[arg(long, default_value_t = false)]
    pub print_build: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Subcommand)]
pub enum DaemonCommand {
    /// Install the envd build published by the configured gateway.
    Upgrade,
}

#[derive(Clone, Debug)]
pub struct DaemonConfig {
    /// Passive listener, when enabled.
    pub listen: Option<SocketAddr>,
    pub cwd: PathBuf,
    pub fs_root: PathBuf,
    pub state_dir: PathBuf,
    pub read_only_fs: bool,
    /// Outbound registration, when a gateway URL is configured.
    pub registration: Option<RegistrationConfig>,
    /// Environment variable names removed from every child process and job.
    pub scrubbed_env: Vec<String>,
}

/// Outbound registration settings. Identity mode is not here on purpose:
/// it is registration-key policy, and the daemon behaves the same either way.
#[derive(Clone)]
pub struct RegistrationConfig {
    pub gateway_url: String,
    /// Present only until the first successful registration consumes it.
    pub registration_key: Option<SecretString>,
    pub display_name: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub receipt_path: Option<PathBuf>,
    pub ca_file: Option<PathBuf>,
    /// Absolute discovery URL reported on mismatch and used by automatic
    /// upgrade.
    pub discovery_url: String,
    /// Present only when mismatch-triggered replacement and re-exec is opted
    /// in. The argument vector is captured before the daemon starts serving.
    pub(crate) auto_upgrade: Option<RestartInvocation>,
}

#[derive(Clone)]
pub(crate) struct RestartInvocation {
    pub(crate) executable: PathBuf,
    pub(crate) arg0: OsString,
    pub(crate) args: Vec<OsString>,
}

impl std::fmt::Debug for RegistrationConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistrationConfig")
            .field("gateway_url", &self.gateway_url)
            .field(
                "registration_key",
                &self.registration_key.as_ref().map(|_| "<redacted>"),
            )
            .field("display_name", &self.display_name)
            .field("metadata", &self.metadata)
            .field("receipt_path", &self.receipt_path)
            .field("ca_file", &self.ca_file)
            .field("discovery_url", &self.discovery_url)
            .field("auto_upgrade", &self.auto_upgrade.is_some())
            .finish()
    }
}

impl RegistrationConfig {
    pub fn validate(&self) -> Result<()> {
        validate_gateway_url(&self.gateway_url)?;
        validate_registration_metadata(self.display_name.as_deref(), &self.metadata)
            .map_err(|message| anyhow::anyhow!("registration metadata: {message}"))?;
        if let Some(key) = &self.registration_key
            && (key.is_empty() || key.expose().chars().any(char::is_whitespace))
        {
            bail!("registration key must be a single non-empty token");
        }
        crate::upgrade::validate_discovery_url(&self.discovery_url)?;
        Ok(())
    }
}

/// Plain `ws://` is accepted only toward loopback; everything else must be
/// `wss://` so the registration key never crosses a network in the clear.
pub fn validate_gateway_url(url: &str) -> Result<()> {
    let rest = if let Some(rest) = url.strip_prefix("wss://") {
        rest
    } else if let Some(rest) = url.strip_prefix("ws://") {
        let host = rest
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default()
            .rsplit_once(':')
            .map(|(host, _)| host)
            .unwrap_or(rest.split(['/', '?', '#']).next().unwrap_or_default());
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let loopback = host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        if !loopback {
            bail!("gateway URL must use wss:// except toward loopback: {url}");
        }
        rest
    } else {
        bail!("gateway URL must start with wss:// (or ws:// toward loopback): {url}");
    };
    if rest.is_empty() || rest.starts_with('/') {
        bail!("gateway URL has no host: {url}");
    }
    Ok(())
}

/// Names of every daemon configuration variable present in the process
/// environment, captured before any child can inherit them.
pub fn scrubbed_env_names() -> Vec<String> {
    let mut names: Vec<String> = std::env::vars_os()
        .filter_map(|(name, _)| name.into_string().ok())
        .filter(|name| name.starts_with(ENV_PREFIX))
        .collect();
    names.sort();
    names
}

impl DaemonArgs {
    pub fn upgrade_request(&self) -> Result<crate::upgrade::UpgradeRequest> {
        let discovery_url = crate::upgrade::resolve_discovery_url(
            self.gateway_url.as_deref(),
            self.discovery_url.as_deref(),
        )?;
        Ok(crate::upgrade::UpgradeRequest {
            discovery_url,
            ca_file: self.ca_file.clone(),
            install_path: started_executable().context("find the started envd executable")?,
            target: release_info::TARGET.to_owned(),
            expected_protocol: None,
        })
    }

    pub fn into_config(self) -> Result<DaemonConfig> {
        let cwd = canonical_dir(
            self.cwd
                .unwrap_or(std::env::current_dir().context("read current directory")?),
            "cwd",
        )?;
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
            Some(path) if path.is_absolute() => path,
            Some(path) => cwd.join(path),
            None => cwd.join(".lightspeed-envd"),
        };
        let scrubbed_env = scrubbed_env_names();
        let registration = match self.gateway_url {
            Some(gateway_url) => {
                let discovery_url = crate::upgrade::resolve_discovery_url(
                    Some(&gateway_url),
                    self.discovery_url.as_deref(),
                )?;
                let auto_upgrade = self
                    .auto_upgrade
                    .then(|| -> Result<RestartInvocation> {
                        Ok(RestartInvocation {
                            executable: started_executable()
                                .context("find the started envd executable")?,
                            arg0: std::env::args_os()
                                .next()
                                .ok_or_else(|| anyhow::anyhow!("process has no argv[0]"))?,
                            args: std::env::args_os().skip(1).collect(),
                        })
                    })
                    .transpose()?;
                let reexec_registration_key = std::env::var(REEXEC_REGISTRATION_KEY_ENV).ok();
                let registration_key = if let Some(key) = reexec_registration_key {
                    Some(SecretString::new(key))
                } else {
                    match (self.registration_key, self.registration_key_file) {
                        (Some(_), Some(_)) => {
                            bail!("set either the registration key or the key file, not both")
                        }
                        (Some(key), None) => Some(SecretString::new(key.trim().to_owned())),
                        (None, Some(path)) => Some(SecretString::new(
                            std::fs::read_to_string(&path)
                                .with_context(|| {
                                    format!("read registration key file {}", path.display())
                                })?
                                .trim()
                                .to_owned(),
                        )),
                        (None, None) => None,
                    }
                };
                let metadata = match self.registration_metadata.as_deref() {
                    Some(json) if !json.trim().is_empty() => {
                        serde_json::from_str::<BTreeMap<String, String>>(json)
                            .context("registration metadata must be a JSON object of strings")?
                    }
                    _ => BTreeMap::new(),
                };
                let registration = RegistrationConfig {
                    gateway_url,
                    registration_key,
                    display_name: self.registration_name.filter(|name| !name.is_empty()),
                    metadata,
                    receipt_path: self.registration_receipt,
                    ca_file: self.ca_file,
                    discovery_url,
                    auto_upgrade,
                };
                registration.validate()?;
                Some(registration)
            }
            None => {
                if self.registration_key.is_some() || self.registration_key_file.is_some() {
                    bail!("a registration key needs LIGHTSPEED_ENVD_GATEWAY_URL to dial");
                }
                if self.auto_upgrade {
                    bail!("automatic upgrade needs LIGHTSPEED_ENVD_GATEWAY_URL to dial");
                }
                None
            }
        };
        let listen = match (self.listen, registration.is_some()) {
            (Some(listen), _) => Some(listen),
            (None, false) => Some(DEFAULT_LISTEN.parse().expect("valid default listener")),
            (None, true) => None,
        };
        forget_secret_env();
        Ok(DaemonConfig {
            listen,
            cwd,
            fs_root,
            state_dir,
            read_only_fs: self.read_only_fs,
            registration,
            scrubbed_env,
        })
    }
}

/// Resolve argv[0] once, before serving anything, so replacement and re-exec
/// use the path the operator or service manager launched rather than a later
/// PATH lookup. Preserve a symlink at that path: the atomic replacement is of
/// the configured executable entry itself.
fn started_executable() -> Result<PathBuf> {
    let argv0 = std::env::args_os()
        .next()
        .ok_or_else(|| anyhow::anyhow!("process has no argv[0]"))?;
    let path = PathBuf::from(&argv0);
    if path.is_absolute() {
        return Ok(path);
    }
    if path.components().count() > 1 {
        return Ok(std::env::current_dir()
            .context("read current directory")?
            .join(path));
    }
    if let Some(search_path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&search_path) {
            let candidate = directory.join(&path);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    std::env::current_exe().context("fall back to the running executable path")
}

/// Drop the secret-bearing variables from this process's environment as soon
/// as they have been read. Children additionally get every daemon variable
/// removed at spawn; this narrows what an in-process bug could leak.
fn forget_secret_env() {
    for name in [
        "LIGHTSPEED_ENVD_REGISTRATION_KEY",
        "LIGHTSPEED_ENVD_REGISTRATION_KEY_FILE",
        REEXEC_REGISTRATION_KEY_ENV,
    ] {
        // SAFETY: called from configuration parsing on the main thread before
        // the daemon spawns any thread that reads the environment.
        unsafe { std::env::remove_var(name) };
    }
}

fn native_filesystem_root(path: &Path) -> PathBuf {
    path.ancestors()
        .last()
        .expect("an absolute canonical path has a filesystem root")
        .to_path_buf()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(temp: &tempfile::TempDir) -> DaemonArgs {
        DaemonArgs {
            command: None,
            listen: None,
            gateway_url: None,
            discovery_url: None,
            auto_upgrade: false,
            registration_key: None,
            registration_key_file: None,
            registration_name: None,
            registration_metadata: None,
            registration_receipt: None,
            ca_file: None,
            cwd: Some(temp.path().to_path_buf()),
            fs_root: Some(temp.path().to_path_buf()),
            state_dir: Some(temp.path().join("state")),
            read_only_fs: false,
            print_build: false,
        }
    }

    #[test]
    fn listener_config_needs_no_identity_or_secret() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config = DaemonArgs {
            listen: Some("127.0.0.1:0".parse().unwrap()),
            ..args(&temp)
        }
        .into_config()
        .expect("config");
        assert_eq!(config.listen.map(|listen| listen.port()), Some(0));
        assert!(config.registration.is_none());
        let defaulted = args(&temp).into_config().expect("config");
        assert_eq!(
            defaulted.listen,
            Some(DEFAULT_LISTEN.parse().expect("default"))
        );
    }

    #[test]
    fn outbound_config_reads_the_key_once_and_disables_the_listener() {
        let temp = tempfile::tempdir().expect("tempdir");
        let key_file = temp.path().join("key");
        std::fs::write(&key_file, "lsrk_secret\n").expect("write key");
        let config = DaemonArgs {
            gateway_url: Some("wss://gateway.example/environment-gateway/connect".to_owned()),
            registration_key_file: Some(key_file),
            registration_name: Some("worker-1".to_owned()),
            registration_metadata: Some(r#"{"harbor.trialId":"t-1"}"#.to_owned()),
            ..args(&temp)
        }
        .into_config()
        .expect("config");
        assert!(config.listen.is_none());
        let registration = config.registration.expect("registration");
        assert_eq!(
            registration
                .registration_key
                .as_ref()
                .map(|key| key.expose()),
            Some("lsrk_secret")
        );
        assert_eq!(registration.display_name.as_deref(), Some("worker-1"));
        assert_eq!(
            registration
                .metadata
                .get("harbor.trialId")
                .map(String::as_str),
            Some("t-1")
        );
        assert!(!format!("{registration:?}").contains("lsrk_secret"));

        let both = DaemonArgs {
            gateway_url: Some("wss://gateway.example/environment-gateway/connect".to_owned()),
            registration_key: Some("lsrk_a".to_owned()),
            registration_key_file: Some(temp.path().join("missing")),
            ..args(&temp)
        }
        .into_config();
        assert!(both.is_err());

        let key_without_gateway = DaemonArgs {
            registration_key: Some("lsrk_a".to_owned()),
            ..args(&temp)
        }
        .into_config();
        assert!(key_without_gateway.is_err());

        let reserved = DaemonArgs {
            gateway_url: Some("wss://gateway.example/environment-gateway/connect".to_owned()),
            registration_metadata: Some(r#"{"lightspeed.x":"y"}"#.to_owned()),
            ..args(&temp)
        }
        .into_config();
        assert!(reserved.is_err());
    }

    #[test]
    fn plain_websocket_gateway_urls_are_loopback_only() {
        assert!(validate_gateway_url("wss://gateway.example/environment-gateway/connect").is_ok());
        assert!(validate_gateway_url("ws://127.0.0.1:18080/environment-gateway/connect").is_ok());
        assert!(validate_gateway_url("ws://localhost:18080/x").is_ok());
        assert!(validate_gateway_url("ws://[::1]:18080/x").is_ok());
        assert!(validate_gateway_url("ws://gateway.example/x").is_err());
        assert!(validate_gateway_url("http://gateway.example/x").is_err());
        assert!(validate_gateway_url("wss:///x").is_err());
    }

    #[test]
    fn upgrade_subcommand_accepts_global_gateway_and_discovery_options() {
        let parsed = DaemonArgs::try_parse_from([
            "lightspeed-envd",
            "upgrade",
            "--gateway-url",
            "wss://gateway.example/environment-gateway/connect",
            "--discovery-url",
            "https://downloads.example/envd.json",
        ])
        .expect("upgrade args");
        assert_eq!(parsed.command, Some(DaemonCommand::Upgrade));
        assert_eq!(
            parsed.gateway_url.as_deref(),
            Some("wss://gateway.example/environment-gateway/connect")
        );
        assert_eq!(
            parsed.discovery_url.as_deref(),
            Some("https://downloads.example/envd.json")
        );
    }

    #[test]
    fn outbound_upgrade_configuration_derives_discovery_and_captures_restart() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config = DaemonArgs {
            gateway_url: Some(
                "wss://gateway.example/environment-gateway/connect?ignored=1".to_owned(),
            ),
            auto_upgrade: true,
            ..args(&temp)
        }
        .into_config()
        .expect("config");
        let registration = config.registration.expect("registration");
        assert_eq!(
            registration.discovery_url,
            "https://gateway.example/.well-known/lightspeed-envd"
        );
        let restart = registration.auto_upgrade.expect("automatic upgrade");
        assert!(restart.executable.is_absolute());

        let parsed = DaemonArgs::try_parse_from([
            "lightspeed-envd",
            "--gateway-url",
            "wss://gateway.example/environment-gateway/connect",
            "--auto-upgrade",
        ])
        .expect("automatic upgrade args");
        assert!(parsed.auto_upgrade);
    }
}
