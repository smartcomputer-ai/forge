use std::collections::{BTreeMap, BTreeSet};

use host_protocol::{
    control::targets::{HostTargetStatus, HostTargetSummary},
    shared::{
        HostCapabilities, HostConnectionSpec, HostPath, HostScope, HostTargetId, HostTransport,
        ImplementationInfo,
    },
};

use super::*;

fn provider() -> RegisterEnvironmentProvider {
    RegisterEnvironmentProvider {
        provider_id: EnvironmentProviderId::new("bridge"),
        provider_kind: EnvironmentProviderKind::Bridge,
        display_name: None,
        controller_connection: HostControllerConnectionSpec::new(
            "http://bridge",
            HostTransport::Http,
        ),
        capabilities: EnvironmentProviderCapabilities {
            list_targets: true,
            create_target: true,
            get_target: true,
            close_target: true,
        },
        implementation: ImplementationInfo {
            name: "test".to_owned(),
            version: None,
        },
        lease_ttl_ms: 1_000,
        metadata: BTreeMap::new(),
        observed_at_ms: 100,
    }
}

fn observation(environment_id: &str, origin: EnvironmentOrigin) -> ObserveEnvironment {
    let target_id = HostTargetId::new("local");
    ObserveEnvironment::from_observation(
        EnvironmentId::new(environment_id),
        EnvironmentProviderId::new("bridge"),
        origin,
        ObservedEnvironmentTarget {
            target: HostTargetSummary {
                target_id: target_id.clone(),
                display_name: None,
                status: HostTargetStatus::Ready,
                scope: HostScope::Default,
                capabilities: HostCapabilities::filesystem(true, true)
                    .with_process()
                    .with_jobs(),
                default_cwd: Some(HostPath::new("/workspace").expect("path")),
                metadata: BTreeMap::new(),
            },
            connection: HostConnectionSpec {
                target_id,
                endpoint: "http://host".to_owned(),
                transport: HostTransport::Http,
                scope: HostScope::Default,
                default_cwd: Some(HostPath::new("/workspace").expect("path")),
                capabilities: HostCapabilities::filesystem(true, true)
                    .with_process()
                    .with_jobs(),
            },
        },
        200,
    )
}

#[test]
fn provider_presence_derives_stale_from_lease() {
    let record = provider().into_record().expect("provider");
    assert_eq!(record.presence_at(500), EnvironmentProviderPresence::Online);
    assert_eq!(
        record.presence_at(1_100),
        EnvironmentProviderPresence::Stale
    );
}

#[tokio::test(flavor = "current_thread")]
async fn provider_target_identity_is_stable_across_observations() {
    let store = InMemoryEnvironmentRegistryStore::new();
    store.register_provider(provider()).await.expect("provider");
    let first = store
        .observe_environment(observation("instance-a", EnvironmentOrigin::Provided))
        .await
        .expect("first");
    let second = store
        .observe_environment(observation("instance-b", EnvironmentOrigin::Provided))
        .await
        .expect("second");
    assert_eq!(first.environment_id, second.environment_id);
}

#[tokio::test(flavor = "current_thread")]
async fn credentials_are_bound_directly_to_universe_environments() {
    let store = InMemoryEnvironmentRegistryStore::new();
    store.register_provider(provider()).await.expect("provider");
    let first = store
        .observe_environment(observation("instance-a", EnvironmentOrigin::Provided))
        .await
        .expect("first");
    store
        .bind_credential(PutEnvironmentCredential {
            environment_id: first.environment_id.clone(),
            env_name: "TOKEN".to_owned(),
            source: EnvironmentCredentialSource::DirectSecret {
                secret_id: SecretId::new("secret"),
            },
            created_at_ms: 301,
        })
        .await
        .expect("credential");
    let credentials = store
        .list_credentials(ListEnvironmentCredentials {
            environment_id: first.environment_id,
        })
        .await
        .expect("credentials");
    assert_eq!(credentials.len(), 1);
    assert_eq!(credentials[0].env_name, "TOKEN");
}

#[tokio::test(flavor = "current_thread")]
async fn close_allows_instances_without_attached_bindings() {
    let store = InMemoryEnvironmentRegistryStore::new();
    store.register_provider(provider()).await.expect("provider");
    let instance = store
        .observe_environment(observation("instance", EnvironmentOrigin::Provisioned))
        .await
        .expect("instance");
    let closing = store
        .begin_close_environment(BeginCloseEnvironment {
            environment_id: instance.environment_id,
            updated_at_ms: 400,
        })
        .await
        .expect("close begins without local job occupancy state");
    assert_eq!(closing.status, HostTargetStatus::Closing);
}

#[tokio::test(flavor = "current_thread")]
async fn missing_provided_targets_become_unknown() {
    let store = InMemoryEnvironmentRegistryStore::new();
    store.register_provider(provider()).await.expect("provider");
    let instance = store
        .observe_environment(observation("instance", EnvironmentOrigin::Provided))
        .await
        .expect("instance");
    let changed = store
        .mark_missing_provided_environments_unknown(
            &EnvironmentProviderId::new("bridge"),
            &BTreeSet::new(),
            500,
        )
        .await
        .expect("missing");
    assert_eq!(changed.len(), 1);
    assert_eq!(
        store
            .read_environment(&instance.environment_id)
            .await
            .expect("instance")
            .status,
        HostTargetStatus::Unknown
    );
}
