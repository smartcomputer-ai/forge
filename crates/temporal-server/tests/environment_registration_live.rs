//! Key-based outbound registration, end to end against the real gateway:
//! an in-process `lightspeed-envd` dials the environment gateway with a
//! registration key, becomes a `Ready` registered environment, serves a
//! worker route through a reverse-dialed data socket, goes `Offline` when
//! its control connection drops, reconnects by identity alone, keeps
//! reconnecting after its key is revoked, and is refused for good once its
//! environment is closed.

mod support;

use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};

use api::{
    AgentApiService, EnvironmentCloseParams, EnvironmentIdentityModeView,
    EnvironmentLifecycleStatusView, EnvironmentListParams, EnvironmentReadParams,
    EnvironmentRegistrationKeyCreateParams, EnvironmentRegistrationKeyReadParams,
    EnvironmentRegistrationKeyRevokeParams, EnvironmentSourceView, EnvironmentView,
    OperatorApiService, OperatorUniverseCreateParams,
};
use environment_client::EnvironmentDataClient;
use environment_daemon::{
    DaemonRuntime,
    config::{DaemonConfig, RegistrationConfig},
};
use environment_protocol::{
    data::{
        fs::{ReadDirectoryParams, WriteFileParams},
        handshake::InitializeParams,
    },
    shared::{ByteChunk, CURRENT_PROTOCOL_VERSION, EnvironmentPath, SecretString},
};
use environments::EnvironmentStore as _;
use support::live::{LIVE_TEST_LOCK, require_storage_live_env};
use temporal_server::{
    DeploymentStores, GatewayAuthMode, UniverseRuntime,
    gateway::{
        DEFAULT_MAX_REQUEST_BODY_BYTES, GatewayAgentApi, GatewayOperatorApi, GatewayRoutes,
        GatewayState, gateway_router,
    },
};
use temporal_workflow::{DEFAULT_TEMPORAL_NAMESPACE, DEFAULT_TEMPORAL_TARGET, connect_temporal};
use uuid::Uuid;

const POLL: Duration = Duration::from_millis(200);
const SETTLE: Duration = Duration::from_secs(30);

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn registered_envd_dials_out_serves_routes_reconnects_and_is_spent_on_close()
-> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;
    let _guard = LIVE_TEST_LOCK.lock().await;

    let temporal_target =
        std::env::var("TEMPORAL_ADDRESS").unwrap_or_else(|_| DEFAULT_TEMPORAL_TARGET.to_owned());
    let namespace = std::env::var("TEMPORAL_NAMESPACE")
        .unwrap_or_else(|_| DEFAULT_TEMPORAL_NAMESPACE.to_owned());
    let client = connect_temporal(&temporal_target, &namespace).await?;
    let stores = DeploymentStores::from_env().await?;
    let suffix = Uuid::new_v4().simple().to_string();
    let universe_id = Uuid::new_v4();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base_url = format!("http://{}", listener.local_addr()?);
    let connect_url = format!(
        "ws://{}/environment-gateway/connect",
        listener.local_addr()?
    );
    let runtime = Arc::new(UniverseRuntime::new(
        client,
        format!("environment-registration-live-{suffix}"),
        Some(base_url.clone()),
        stores,
    )?);
    let operator = GatewayOperatorApi::new(runtime.clone());
    operator
        .create_universe(OperatorUniverseCreateParams {
            universe_id: universe_id.to_string(),
        })
        .await?;
    let state = Arc::new(GatewayState::multi(
        GatewayAuthMode::Single { universe_id },
        runtime.clone(),
        base_url.clone(),
    ));
    let gateway = tokio::spawn(async move {
        axum::serve(
            listener,
            gateway_router(state, DEFAULT_MAX_REQUEST_BODY_BYTES, GatewayRoutes::ALL),
        )
        .await
    });
    // A process without the environment-gateway role has no registration
    // routes at all: a misrouted daemon fails loudly with a 404 instead of
    // registering somewhere no worker can reach.
    let api_only_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let api_only_url = format!("http://{}", api_only_listener.local_addr()?);
    let api_only_state = Arc::new(GatewayState::multi(
        GatewayAuthMode::Single { universe_id },
        runtime.clone(),
        api_only_url.clone(),
    ));
    let api_only = tokio::spawn(async move {
        axum::serve(
            api_only_listener,
            gateway_router(
                api_only_state,
                DEFAULT_MAX_REQUEST_BODY_BYTES,
                GatewayRoutes {
                    api: true,
                    environment: false,
                },
            ),
        )
        .await
    });
    let http = reqwest::Client::new();
    for path in ["/environment-gateway/connect", "/environment-gateway/data"] {
        let status = http
            .get(format!("{api_only_url}{path}"))
            .send()
            .await?
            .status();
        assert_eq!(
            status,
            reqwest::StatusCode::NOT_FOUND,
            "{path} on an api-only process"
        );
    }
    assert_eq!(
        http.get(format!("{api_only_url}/health"))
            .send()
            .await?
            .status(),
        reqwest::StatusCode::OK
    );
    assert_ne!(
        http.get(format!("{base_url}/environment-gateway/connect"))
            .send()
            .await?
            .status(),
        reqwest::StatusCode::NOT_FOUND,
        "the environment-gateway process serves the connect route"
    );
    api_only.abort();
    let reconciler = tokio::spawn(runtime.clone().run_environment_reconciler());
    let api = runtime.state_for(universe_id, false).await?.api.clone();
    let sandbox = tempfile::tempdir()?;

    let result = scenario(
        &runtime,
        &api,
        universe_id,
        &base_url,
        &connect_url,
        sandbox.path(),
    )
    .await;
    reconciler.abort();
    gateway.abort();
    result
}

async fn scenario(
    runtime: &Arc<UniverseRuntime>,
    api: &Arc<GatewayAgentApi>,
    universe_id: Uuid,
    base_url: &str,
    connect_url: &str,
    sandbox: &Path,
) -> anyhow::Result<()> {
    let minted = api
        .create_environment_registration_key(EnvironmentRegistrationKeyCreateParams {
            display_name: "registration live pool".to_owned(),
            identity_mode: EnvironmentIdentityModeView::Ephemeral,
            max_active_environments: Some(2),
            ephemeral_disconnect_grace_ms: None,
            expires_at_ms: None,
        })
        .await?
        .result;
    let key_id = minted.registration_key.registration_key_id.clone();
    let secret = minted.secret.0.clone();
    assert!(secret.starts_with("lsrk_"));
    assert_eq!(minted.registration_key.active_environment_count, 0);

    // First daemon: a fresh identity admitted by the key.
    let root_a = sandbox.join("a");
    std::fs::create_dir_all(&root_a)?;
    let receipt_a = root_a.join("receipt.json");
    let daemon_a = spawn_daemon(daemon_config(
        &root_a,
        connect_url,
        Some(&secret),
        Some(&receipt_a),
    )?);
    let environment_a =
        wait_for_registered(api, &key_id, 1, EnvironmentLifecycleStatusView::Ready).await?;
    let EnvironmentSourceView::Registered {
        registration_key_id,
        daemon_id,
        identity_mode,
    } = &environment_a.source
    else {
        anyhow::bail!(
            "expected a registered source, got {:?}",
            environment_a.source
        )
    };
    assert_eq!(registration_key_id, &key_id);
    assert_eq!(*identity_mode, EnvironmentIdentityModeView::Ephemeral);
    assert!(daemon_id.starts_with("daemon_"));
    assert!(environment_a.last_seen_at_ms.is_some());
    assert_eq!(
        environment_a
            .metadata
            .get("harbor.trialId")
            .map(String::as_str),
        Some("trial-1")
    );
    assert!(
        environment_a
            .metadata
            .contains_key("lightspeed.envd.version")
    );
    let receipt: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&receipt_a)?)?;
    assert_eq!(receipt["environmentId"], environment_a.environment_id);
    assert_eq!(receipt["daemonId"], daemon_id.as_str());
    assert_eq!(receipt["identityMode"], "ephemeral");
    assert!(receipt.get("registrationKey").is_none());
    let key_view = api
        .read_environment_registration_key(EnvironmentRegistrationKeyReadParams {
            registration_key_id: key_id.clone(),
        })
        .await?
        .result
        .registration_key;
    assert_eq!(key_view.active_environment_count, 1);
    assert_eq!(key_view.registered_environment_count, 1);

    // The worker route reaches the daemon through a reverse-dialed data
    // socket; two concurrent routes each get their own socket.
    let record = runtime
        .state_for(universe_id, false)
        .await?
        .store
        .read_environment(&environments::EnvironmentId::new(
            environment_a.environment_id.clone(),
        ))
        .await?;
    // Workers reach this test's gateway, not whatever the sourced dev
    // environment points LIGHTSPEED_ENVIRONMENT_GATEWAY_URL at; the route
    // bearer is the deployment token the gateway state checks.
    let gateway = temporal_server::environment_gateway::EnvironmentGatewayClientConfig::new(
        base_url,
        runtime.environment_gateway().deployment_token(),
    );
    let connection = gateway.connection_for(universe_id, &record);
    let mut first = EnvironmentDataClient::connect(
        &connection.endpoint,
        gateway.connect_options("registration-live"),
    )
    .await?;
    let mut second = EnvironmentDataClient::connect(
        &connection.endpoint,
        gateway.connect_options("registration-live"),
    )
    .await?;
    for client in [&mut first, &mut second] {
        let initialized = client
            .initialize(&InitializeParams {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                client_name: "registration-live".to_owned(),
                scope: environment_protocol::shared::EnvironmentScope::Default,
                resume_connection_id: None,
            })
            .await?;
        assert_eq!(initialized.protocol_version, CURRENT_PROTOCOL_VERSION);
    }
    first
        .write_file(&WriteFileParams {
            path: EnvironmentPath::new("registered.txt")?,
            data: ByteChunk::new(b"through the reverse dial".to_vec()),
        })
        .await?;
    let listing = second
        .read_directory(&ReadDirectoryParams {
            path: EnvironmentPath::new(".")?,
        })
        .await?;
    assert!(
        listing
            .entries
            .iter()
            .any(|entry| entry.file_name == "registered.txt"),
        "second data socket sees the file the first wrote: {listing:?}"
    );
    assert_eq!(
        std::fs::read_to_string(root_a.join("registered.txt"))?,
        "through the reverse dial"
    );
    first.close().await?;
    second.close().await?;

    // Dropping the control connection makes the environment Offline; the
    // same identity, now without any key, reconnects the same environment.
    daemon_a.abort();
    wait_for_status(
        api,
        &environment_a.environment_id,
        EnvironmentLifecycleStatusView::Offline,
    )
    .await?;
    let daemon_a = spawn_daemon(daemon_config(&root_a, connect_url, None, Some(&receipt_a))?);
    wait_for_status(
        api,
        &environment_a.environment_id,
        EnvironmentLifecycleStatusView::Ready,
    )
    .await?;
    let receipt: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&receipt_a)?)?;
    assert_eq!(receipt["environmentId"], environment_a.environment_id);
    assert_eq!(
        wait_for_registered(api, &key_id, 1, EnvironmentLifecycleStatusView::Ready)
            .await?
            .environment_id,
        environment_a.environment_id,
        "reconnect must not create a second environment"
    );

    // A second identity with the same key is a second environment.
    let root_b = sandbox.join("b");
    std::fs::create_dir_all(&root_b)?;
    let daemon_b = spawn_daemon(daemon_config(&root_b, connect_url, Some(&secret), None)?);
    let environments = wait_for_registered_count(api, &key_id, 2).await?;
    let environment_b = environments
        .iter()
        .find(|environment| environment.environment_id != environment_a.environment_id)
        .cloned()
        .expect("second environment");
    assert_eq!(environment_b.status, EnvironmentLifecycleStatusView::Ready);

    // Revocation without cascade stops new identities but not reconnects.
    api.revoke_environment_registration_key(EnvironmentRegistrationKeyRevokeParams {
        registration_key_id: key_id.clone(),
        close_environments: false,
    })
    .await?;
    let root_c = sandbox.join("c");
    std::fs::create_dir_all(&root_c)?;
    let daemon_c = spawn_daemon(daemon_config(&root_c, connect_url, Some(&secret), None)?);
    let refused = tokio::time::timeout(SETTLE, daemon_c).await??;
    let refused = refused.expect_err("a revoked key is a terminal refusal for a new daemon");
    assert!(
        refused.to_string().contains("RegistrationKeyRevoked"),
        "unexpected refusal: {refused:#}"
    );
    daemon_b.abort();
    wait_for_status(
        api,
        &environment_b.environment_id,
        EnvironmentLifecycleStatusView::Offline,
    )
    .await?;
    let daemon_b = spawn_daemon(daemon_config(&root_b, connect_url, None, None)?);
    wait_for_status(
        api,
        &environment_b.environment_id,
        EnvironmentLifecycleStatusView::Ready,
    )
    .await?;

    // Closing the environment spends the identity: the daemon is told to go
    // and its reconnect is refused for good.
    api.close_environment(EnvironmentCloseParams {
        environment_id: environment_a.environment_id.clone(),
    })
    .await?;
    let spent = tokio::time::timeout(SETTLE, daemon_a).await??;
    let spent = spent.expect_err("a closed environment is a terminal refusal");
    assert!(
        spent.to_string().contains("EnvironmentClosed"),
        "unexpected refusal: {spent:#}"
    );
    wait_for_status(
        api,
        &environment_a.environment_id,
        EnvironmentLifecycleStatusView::Closed,
    )
    .await?;
    let key_view = api
        .read_environment_registration_key(EnvironmentRegistrationKeyReadParams {
            registration_key_id: key_id.clone(),
        })
        .await?
        .result
        .registration_key;
    assert_eq!(key_view.registered_environment_count, 2);
    assert_eq!(key_view.active_environment_count, 1);
    assert_eq!(
        key_view.status,
        api::EnvironmentRegistrationKeyStatusView::Revoked
    );

    daemon_b.abort();
    api.close_environment(EnvironmentCloseParams {
        environment_id: environment_b.environment_id.clone(),
    })
    .await?;
    wait_for_status(
        api,
        &environment_b.environment_id,
        EnvironmentLifecycleStatusView::Closed,
    )
    .await?;

    // Ephemeral grace: a key with a short grace closes an environment whose
    // daemon stays away, through the lifecycle reconciler alone.
    let short = api
        .create_environment_registration_key(EnvironmentRegistrationKeyCreateParams {
            display_name: "registration live short grace".to_owned(),
            identity_mode: EnvironmentIdentityModeView::Ephemeral,
            max_active_environments: None,
            ephemeral_disconnect_grace_ms: Some(1_500),
            expires_at_ms: None,
        })
        .await?
        .result;
    let short_key_id = short.registration_key.registration_key_id.clone();
    let root_d = sandbox.join("d");
    std::fs::create_dir_all(&root_d)?;
    let daemon_d = spawn_daemon(daemon_config(
        &root_d,
        connect_url,
        Some(&short.secret.0),
        None,
    )?);
    let environment_d =
        wait_for_registered(api, &short_key_id, 1, EnvironmentLifecycleStatusView::Ready).await?;
    daemon_d.abort();
    wait_for_status(
        api,
        &environment_d.environment_id,
        EnvironmentLifecycleStatusView::Closed,
    )
    .await?;
    let short_view = api
        .read_environment_registration_key(EnvironmentRegistrationKeyReadParams {
            registration_key_id: short_key_id,
        })
        .await?
        .result
        .registration_key;
    assert_eq!(short_view.active_environment_count, 0);
    assert_eq!(short_view.registered_environment_count, 1);
    Ok(())
}

fn daemon_config(
    root: &Path,
    connect_url: &str,
    key: Option<&str>,
    receipt: Option<&Path>,
) -> anyhow::Result<DaemonConfig> {
    let root = root.canonicalize()?;
    let mut metadata = BTreeMap::new();
    metadata.insert("harbor.trialId".to_owned(), "trial-1".to_owned());
    Ok(DaemonConfig {
        listen: None,
        cwd: root.clone(),
        fs_root: root.clone(),
        state_dir: root.join(".lightspeed-envd"),
        read_only_fs: false,
        registration: Some(RegistrationConfig {
            gateway_url: connect_url.to_owned(),
            registration_key: key.map(SecretString::new),
            display_name: Some("registration live daemon".to_owned()),
            metadata,
            receipt_path: receipt.map(Path::to_path_buf),
            ca_file: None,
        }),
        scrubbed_env: Vec::new(),
    })
}

fn spawn_daemon(config: DaemonConfig) -> tokio::task::JoinHandle<anyhow::Result<()>> {
    tokio::spawn(async move {
        let runtime = DaemonRuntime::new(config)?;
        environment_daemon::server::run(runtime).await
    })
}

async fn wait_for_registered(
    api: &GatewayAgentApi,
    key_id: &str,
    count: usize,
    status: EnvironmentLifecycleStatusView,
) -> anyhow::Result<EnvironmentView> {
    let deadline = tokio::time::Instant::now() + SETTLE;
    loop {
        let environments = list_by_key(api, key_id).await?;
        if environments.len() == count
            && let Some(environment) = environments
                .iter()
                .find(|environment| environment.status == status)
        {
            return Ok(environment.clone());
        }
        if tokio::time::Instant::now() > deadline {
            anyhow::bail!(
                "timed out waiting for {count} environment(s) under {key_id} with one {status:?}: {environments:?}"
            );
        }
        tokio::time::sleep(POLL).await;
    }
}

async fn wait_for_registered_count(
    api: &GatewayAgentApi,
    key_id: &str,
    count: usize,
) -> anyhow::Result<Vec<EnvironmentView>> {
    let deadline = tokio::time::Instant::now() + SETTLE;
    loop {
        let environments = list_by_key(api, key_id).await?;
        if environments.len() == count
            && environments
                .iter()
                .all(|environment| environment.status == EnvironmentLifecycleStatusView::Ready)
        {
            return Ok(environments);
        }
        if tokio::time::Instant::now() > deadline {
            anyhow::bail!("timed out waiting for {count} ready environments: {environments:?}");
        }
        tokio::time::sleep(POLL).await;
    }
}

async fn list_by_key(api: &GatewayAgentApi, key_id: &str) -> anyhow::Result<Vec<EnvironmentView>> {
    Ok(api
        .list_environments(EnvironmentListParams {
            registration_key_id: Some(key_id.to_owned()),
            ..EnvironmentListParams::default()
        })
        .await?
        .result
        .environments)
}

async fn wait_for_status(
    api: &GatewayAgentApi,
    environment_id: &str,
    status: EnvironmentLifecycleStatusView,
) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + SETTLE;
    loop {
        let environment = api
            .read_environment(EnvironmentReadParams {
                environment_id: environment_id.to_owned(),
            })
            .await?
            .result
            .environment;
        if environment.status == status {
            return Ok(());
        }
        if tokio::time::Instant::now() > deadline {
            anyhow::bail!(
                "timed out waiting for {environment_id} to be {status:?}; last {:?}",
                environment.status
            );
        }
        tokio::time::sleep(POLL).await;
    }
}
