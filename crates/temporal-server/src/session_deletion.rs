use std::time::{SystemTime, UNIX_EPOCH};

use engine::{
    SessionId,
    storage::{DeleteClosedSessions, DeleteClosedSessionsResult, SessionStore, SessionStoreError},
};
use environments::{BeginCloseEnvironment, EnvironmentStatus, EnvironmentStore, ListEnvironments};
use store_pg::PgStore;

#[derive(Clone, Copy, Debug)]
pub(crate) enum SessionDeletionCause {
    Manual,
    Retention,
}

impl SessionDeletionCause {
    fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Retention => "retention",
        }
    }
}

/// Delete a closed session subtree and eagerly close every owned
/// `closeWithSession` environment. Environment cleanup is best effort: the
/// lifecycle reconciler also observes missing origin sessions and converges.
pub(crate) async fn delete_session_subtree(
    store: &PgStore,
    request: DeleteClosedSessions,
    cause: SessionDeletionCause,
) -> Result<DeleteClosedSessionsResult, SessionStoreError> {
    let requested_session_id = request.session_id.clone();
    let cascade = request.cascade;
    let deleted = SessionStore::delete_closed_sessions(store, request).await?;
    for session_id in &deleted.deleted_session_ids {
        close_session_owned_environments(store, session_id).await;
    }
    tracing::info!(
        target: "temporal_server",
        requested_session_id = %requested_session_id,
        retention_root_session_id = %deleted.target.retention_root_session_id,
        deleted_session_count = deleted.deleted_session_ids.len(),
        cascade,
        cause = cause.as_str(),
        "session deletion complete"
    );
    Ok(deleted)
}

async fn close_session_owned_environments(store: &PgStore, session_id: &SessionId) {
    let Ok(environments) = EnvironmentStore::list_environments(
        store,
        ListEnvironments {
            metadata: Default::default(),
            origin_session_id: Some(session_id.clone()),
            ..ListEnvironments::default()
        },
    )
    .await
    else {
        return;
    };
    for environment in environments {
        let should_close = environment
            .origin_session
            .as_ref()
            .is_some_and(|origin| origin.close_with_session)
            && !matches!(
                environment.status,
                EnvironmentStatus::Closing | EnvironmentStatus::Closed
            );
        if !should_close {
            continue;
        }
        let _ = EnvironmentStore::begin_close_environment(
            store,
            BeginCloseEnvironment {
                environment_id: environment.environment_id,
                updated_at_ms: now_ms(),
            },
        )
        .await;
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}
