use std::collections::BTreeMap;

use environment_protocol::shared::{EnvironmentTransport, ProviderTargetId};
use uuid::Uuid;

use super::*;

fn provider() -> PutEnvironmentProvider {
    PutEnvironmentProvider {
        provider_id: EnvironmentProviderId::new("incus-local"),
        display_name: Some("Local Incus".to_owned()),
        controller_connection: EnvironmentConnectionSpec::new(
            "ws://127.0.0.1:19090/control",
            EnvironmentTransport::WebSocket,
        ),
        metadata: BTreeMap::new(),
        updated_at_ms: 1_000,
    }
}

fn binding(universe_id: Uuid, expected_revision: Option<u64>) -> PutEnvironmentProviderBinding {
    PutEnvironmentProviderBinding {
        universe_id,
        binding_id: EnvironmentProviderBindingId::new("primary"),
        provider_id: EnvironmentProviderId::new("incus-local"),
        status: EnvironmentProviderBindingStatus::Enabled,
        metadata: BTreeMap::new(),
        expected_revision,
        updated_at_ms: 1_000 + expected_revision.unwrap_or(0) as i64,
    }
}

fn create(request: &str, environment: &str, incarnation: &str, at: i64) -> CreateEnvironment {
    CreateEnvironment {
        request_id: EnvironmentProvisionRequestId::new(request),
        environment_id: EnvironmentId::new(environment),
        incarnation_id: EnvironmentIncarnationId::new(incarnation),
        binding_id: EnvironmentProviderBindingId::new("primary"),
        template_id: EnvironmentTemplateId::new("rust-v1"),
        display_name: None,
        metadata: BTreeMap::new(),
        origin_session: None,
        created_at_ms: at,
    }
}

async fn store() -> (Uuid, InMemoryEnvironmentRegistryStore) {
    let universe_id = Uuid::new_v4();
    let store = InMemoryEnvironmentRegistryStore::for_universe(universe_id);
    store.put_provider(provider()).await.expect("provider");
    store
        .put_provider_binding(binding(universe_id, None))
        .await
        .expect("binding");
    (universe_id, store)
}

#[tokio::test(flavor = "current_thread")]
async fn binding_put_is_revisioned_and_unique_per_provider() {
    let (universe_id, store) = store().await;
    let first = store
        .read_provider_binding(universe_id, &EnvironmentProviderBindingId::new("primary"))
        .await
        .expect("read");
    assert_eq!(first.revision, 1);

    let conflict = store
        .put_provider_binding(binding(universe_id, None))
        .await
        .expect_err("stale write");
    assert!(matches!(
        conflict,
        EnvironmentRegistryError::RevisionConflict {
            actual: Some(1),
            ..
        }
    ));

    let second = store
        .put_provider_binding(binding(universe_id, Some(1)))
        .await
        .expect("replace");
    assert_eq!(second.revision, 2);

    let mut duplicate = binding(universe_id, None);
    duplicate.binding_id = EnvironmentProviderBindingId::new("secondary");
    assert!(matches!(
        store.put_provider_binding(duplicate).await,
        Err(EnvironmentRegistryError::AlreadyExists { .. })
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn provider_delete_requires_all_bindings_to_be_removed() {
    let (universe_id, store) = store().await;
    let provider_id = EnvironmentProviderId::new("incus-local");
    assert!(matches!(
        store.delete_provider(&provider_id).await,
        Err(EnvironmentRegistryError::InvalidInput { .. })
    ));
    store
        .delete_provider_binding(universe_id, &EnvironmentProviderBindingId::new("primary"))
        .await
        .expect("delete binding");
    let deleted = store
        .delete_provider(&provider_id)
        .await
        .expect("delete provider");
    assert_eq!(deleted.provider_id, provider_id);
    assert!(matches!(
        store.read_provider(&provider_id).await,
        Err(EnvironmentRegistryError::NotFound { .. })
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn stable_request_id_returns_the_original_environment() {
    let (_, store) = store().await;
    let first = store
        .create_environment(create("request-1", "environment-1", "incarnation-1", 2_000))
        .await
        .expect("first");
    let retry = store
        .create_environment(create(
            "request-1",
            "different-environment",
            "different-incarnation",
            3_000,
        ))
        .await
        .expect("retry");
    assert_eq!(retry.environment_id, first.environment_id);
    assert_eq!(
        retry.incarnation.incarnation_id,
        first.incarnation.incarnation_id
    );
    assert_eq!(
        store
            .list_environments(ListEnvironments::default())
            .await
            .expect("list")
            .len(),
        1
    );
}

#[tokio::test(flavor = "current_thread")]
async fn adoption_creates_a_provisioned_environment_without_a_template() {
    let (_, store) = store().await;
    let adopted = store
        .adopt_environment(AdoptEnvironment {
            request_id: EnvironmentProvisionRequestId::new("adopt-request-1"),
            environment_id: EnvironmentId::new("environment-adopted"),
            incarnation_id: EnvironmentIncarnationId::new("incarnation-adopted"),
            binding_id: EnvironmentProviderBindingId::new("primary"),
            source_target: "legacy/hand-built-vm".to_owned(),
            display_name: Some("Hand-built VM".to_owned()),
            metadata: BTreeMap::new(),
            created_at_ms: 2_000,
        })
        .await
        .expect("adopt");

    assert_eq!(adopted.status, EnvironmentStatus::Provisioning);
    assert!(matches!(
        adopted.source,
        EnvironmentSource::Provisioned { .. }
    ));
    assert_eq!(adopted.incarnation.template_id, None);
    assert_eq!(
        adopted.incarnation.adoption_source_target.as_deref(),
        Some("legacy/hand-built-vm")
    );

    let retry = store
        .adopt_environment(AdoptEnvironment {
            request_id: EnvironmentProvisionRequestId::new("adopt-request-1"),
            environment_id: EnvironmentId::new("ignored-on-retry"),
            incarnation_id: EnvironmentIncarnationId::new("ignored-on-retry"),
            binding_id: EnvironmentProviderBindingId::new("primary"),
            source_target: "legacy/another-vm".to_owned(),
            display_name: None,
            metadata: BTreeMap::new(),
            created_at_ms: 3_000,
        })
        .await
        .expect("idempotent retry");
    assert_eq!(retry.environment_id, adopted.environment_id);
}

#[tokio::test(flavor = "current_thread")]
async fn lightspeed_does_not_enforce_provider_quota() {
    let (_, store) = store().await;
    let first = store
        .create_environment(create("request-1", "environment-1", "incarnation-1", 2_000))
        .await
        .expect("first");
    store
        .fail_environment_lifecycle(FailEnvironmentLifecycle {
            environment_id: first.environment_id.clone(),
            message: "provider failed".to_owned(),
            observed_at_ms: 3_000,
        })
        .await
        .expect("fail");
    store
        .create_environment(create("request-2", "environment-2", "incarnation-2", 4_000))
        .await
        .expect("second intent");
}

#[tokio::test(flavor = "current_thread")]
async fn provider_observation_populates_only_the_current_incarnation() {
    let (_, store) = store().await;
    let environment = store
        .create_environment(create("request-1", "environment-1", "incarnation-1", 2_000))
        .await
        .expect("create");
    let target_id = ProviderTargetId::new("target-1");
    let observed = store
        .observe_provisioned_environment(ObserveProvisionedEnvironment {
            environment_id: environment.environment_id.clone(),
            provider_target_id: target_id.clone(),
            status: EnvironmentStatus::Ready,
            observed_at_ms: 3_000,
        })
        .await
        .expect("observe");
    assert_eq!(observed.status, EnvironmentStatus::Ready);
    assert_eq!(observed.incarnation.provider_target_id, Some(target_id));
    assert_eq!(
        observed.incarnation.template_id,
        Some(EnvironmentTemplateId::new("rust-v1"))
    );
}

#[tokio::test(flavor = "current_thread")]
async fn public_ingress_is_a_realized_environment_facet() {
    let (_, store) = store().await;
    let environment = store
        .create_environment(create("request-1", "environment-1", "incarnation-1", 2_000))
        .await
        .expect("create");
    let enabled = store
        .set_environment_ingress(SetEnvironmentIngress {
            environment_id: environment.environment_id.clone(),
            enabled: true,
            public_endpoint: Some("https://opaque.env.example".to_owned()),
            updated_at_ms: 3_000,
        })
        .await
        .expect("enable ingress");
    assert!(enabled.public_ingress_enabled);
    assert_eq!(
        enabled.public_endpoint.as_deref(),
        Some("https://opaque.env.example")
    );
    let disabled = store
        .set_environment_ingress(SetEnvironmentIngress {
            environment_id: environment.environment_id,
            enabled: false,
            public_endpoint: None,
            updated_at_ms: 4_000,
        })
        .await
        .expect("disable ingress");
    assert!(!disabled.public_ingress_enabled);
    assert_eq!(disabled.public_endpoint, None);
}

#[tokio::test(flavor = "current_thread")]
async fn external_environment_cannot_enable_provider_managed_ingress() {
    let store = InMemoryEnvironmentRegistryStore::new();
    let environment = store
        .create_external_environment(CreateExternalEnvironment {
            request_id: EnvironmentProvisionRequestId::new("external-request"),
            environment_id: EnvironmentId::new("external-environment"),
            incarnation_id: EnvironmentIncarnationId::new("external-incarnation"),
            connection: EnvironmentConnectionSpec::new(
                "ws://envd.example",
                EnvironmentTransport::WebSocket,
            ),
            display_name: None,
            metadata: BTreeMap::new(),
            created_at_ms: 1_000,
        })
        .await
        .expect("external");
    assert!(matches!(
        store
            .set_environment_ingress(SetEnvironmentIngress {
                environment_id: environment.environment_id,
                enabled: true,
                public_endpoint: Some("https://invalid.example".to_owned()),
                updated_at_ms: 2_000,
            })
            .await,
        Err(EnvironmentRegistryError::InvalidInput { .. })
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn external_environment_persists_a_typed_connection() {
    let store = InMemoryEnvironmentRegistryStore::new();
    let environment = store
        .create_external_environment(CreateExternalEnvironment {
            request_id: EnvironmentProvisionRequestId::new("external-request"),
            environment_id: EnvironmentId::new("external-environment"),
            incarnation_id: EnvironmentIncarnationId::new("external-incarnation"),
            connection: EnvironmentConnectionSpec::new(
                "ws://envd.example:19091",
                EnvironmentTransport::WebSocket,
            ),
            display_name: None,
            metadata: BTreeMap::new(),
            created_at_ms: 1_000,
        })
        .await
        .expect("create external environment");
    let EnvironmentSource::External { connection } = environment.source else {
        panic!("external source")
    };
    assert_eq!(connection.endpoint, "ws://envd.example:19091");
}

#[tokio::test(flavor = "current_thread")]
async fn disabled_binding_blocks_create_and_live_references_block_delete() {
    let (universe_id, store) = store().await;
    let environment = store
        .create_environment(create("request-1", "environment-1", "incarnation-1", 2_000))
        .await
        .expect("create");
    let mut disabled = binding(universe_id, Some(1));
    disabled.status = EnvironmentProviderBindingStatus::Disabled;
    store.put_provider_binding(disabled).await.expect("disable");
    assert!(matches!(
        store
            .create_environment(create("request-2", "environment-2", "incarnation-2", 3_000))
            .await,
        Err(EnvironmentRegistryError::InvalidInput { .. })
    ));
    assert!(matches!(
        store
            .delete_provider_binding(universe_id, &EnvironmentProviderBindingId::new("primary"))
            .await,
        Err(EnvironmentRegistryError::InvalidInput { .. })
    ));
    store
        .finish_close_environment(FinishCloseEnvironment {
            environment_id: environment.environment_id,
            observed_at_ms: 4_000,
        })
        .await
        .expect("close");
    store
        .delete_provider_binding(universe_id, &EnvironmentProviderBindingId::new("primary"))
        .await
        .expect("delete");
}

#[tokio::test(flavor = "current_thread")]
async fn origin_session_is_recorded_listed_and_swept() {
    let (_universe, store) = store().await;
    let mut request = create("session:s-1", "env-s1", "inc-s1", 1);
    request.origin_session = Some(EnvironmentOriginSession {
        session_id: SessionId::new("s-1"),
        profile_id: Some("coder".to_owned()),
        close_with_session: true,
    });
    let created = store.create_environment(request).await.expect("create");
    assert_eq!(
        created
            .origin_session
            .as_ref()
            .map(|origin| origin.session_id.as_str()),
        Some("s-1")
    );
    let plain = store
        .create_environment(create("plain", "env-plain", "inc-plain", 2))
        .await
        .expect("create plain");
    assert!(plain.origin_session.is_none());

    let by_session = store
        .list_environments(ListEnvironments {
            provider_id: None,
            binding_id: None,
            status: None,
            origin_session_id: Some(SessionId::new("s-1")),
        })
        .await
        .expect("list");
    assert_eq!(by_session.len(), 1);
    assert_eq!(by_session[0].environment_id.as_str(), "env-s1");

    let sweep = store
        .list_environments_closing_with_session()
        .await
        .expect("sweep");
    assert_eq!(sweep.len(), 1);
    assert_eq!(sweep[0].environment_id.as_str(), "env-s1");

    store
        .begin_close_environment(BeginCloseEnvironment {
            environment_id: EnvironmentId::new("env-s1"),
            updated_at_ms: 3,
        })
        .await
        .expect("begin close");
    assert!(
        store
            .list_environments_closing_with_session()
            .await
            .expect("sweep")
            .is_empty()
    );
}

#[test]
fn session_provision_request_id_is_deterministic_and_bounded() {
    let short = SessionId::new("session-1");
    assert_eq!(
        EnvironmentProvisionRequestId::for_session(&short).as_str(),
        "session:session-1"
    );
    assert_eq!(
        EnvironmentProvisionRequestId::for_session(&short),
        EnvironmentProvisionRequestId::for_session(&short)
    );
    let long = SessionId::new("s".repeat(128));
    let derived = EnvironmentProvisionRequestId::for_session(&long);
    assert!(derived.as_str().starts_with("session:sha256-"));
    assert!(derived.as_str().len() <= 128);
    assert_eq!(derived, EnvironmentProvisionRequestId::for_session(&long));
}
