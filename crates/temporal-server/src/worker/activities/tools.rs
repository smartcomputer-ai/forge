use engine::{
    PromiseControlArgumentCallFacts, PromiseControlArgumentFacts, PromiseControlArgumentRequest,
    PromiseControlKind, PromiseId, ToolBatchOutcome, storage::BlobStore,
};
use temporalio_sdk::activities::ActivityError;
use tools::concurrency::{CancelArgs, DetachArgs};

use crate::worker::ToolInvokeBatchActivityRequest;

use super::{
    common::{activity_error, failed_tool_batch_result},
    state::ToolActivityDeps,
};

pub(super) async fn prepare_promise_controls(
    blobs: &dyn BlobStore,
    request: PromiseControlArgumentRequest,
) -> Result<PromiseControlArgumentFacts, ActivityError> {
    if request.version != PromiseControlArgumentRequest::VERSION {
        return Err(activity_error(anyhow::anyhow!(
            "unsupported promise-control argument request version {}",
            request.version
        )));
    }
    let mut calls = Vec::with_capacity(request.calls.len());
    for call in request.calls {
        let parsed = match blobs.read_bytes(&call.arguments_ref).await {
            Ok(bytes) => match call.kind {
                PromiseControlKind::Cancel => serde_json::from_slice::<CancelArgs>(&bytes)
                    .ok()
                    .and_then(|args| args.validated_promise_ids().ok()),
                PromiseControlKind::Detach => serde_json::from_slice::<DetachArgs>(&bytes)
                    .ok()
                    .and_then(|args| args.validated_promise_ids().ok()),
            },
            Err(_) => None,
        };
        calls.push(match parsed {
            Some(promise_ids) => PromiseControlArgumentCallFacts::Parsed {
                call_id: call.call_id,
                promise_ids: promise_ids.into_iter().map(PromiseId::new).collect(),
            },
            None => PromiseControlArgumentCallFacts::Invalid {
                call_id: call.call_id,
            },
        });
    }
    Ok(PromiseControlArgumentFacts {
        version: PromiseControlArgumentFacts::VERSION,
        calls,
    })
}

#[cfg(test)]
mod tests {
    use engine::{BlobRef, PromiseControlArgumentCall, ToolCallId, storage::InMemoryBlobStore};

    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn preparation_reads_only_cas_and_returns_bounded_validated_ids() {
        let blobs = InMemoryBlobStore::new();
        let valid_ref = blobs
            .put_bytes(br#"{"promises":["p1","p1","p2"]}"#.to_vec())
            .await
            .expect("valid args");
        let invalid_ref = blobs
            .put_bytes(br#"{"promises":[]}"#.to_vec())
            .await
            .expect("invalid args");
        let facts = prepare_promise_controls(
            &blobs,
            PromiseControlArgumentRequest {
                version: PromiseControlArgumentRequest::VERSION,
                calls: vec![
                    PromiseControlArgumentCall {
                        call_id: ToolCallId::new("cancel"),
                        kind: PromiseControlKind::Cancel,
                        arguments_ref: valid_ref,
                    },
                    PromiseControlArgumentCall {
                        call_id: ToolCallId::new("detach-invalid"),
                        kind: PromiseControlKind::Detach,
                        arguments_ref: invalid_ref,
                    },
                    PromiseControlArgumentCall {
                        call_id: ToolCallId::new("missing"),
                        kind: PromiseControlKind::Cancel,
                        arguments_ref: BlobRef::from_bytes(b"missing"),
                    },
                ],
            },
        )
        .await
        .expect("prepare facts");

        assert_eq!(
            facts.calls,
            vec![
                PromiseControlArgumentCallFacts::Parsed {
                    call_id: ToolCallId::new("cancel"),
                    promise_ids: vec![PromiseId::new("p1"), PromiseId::new("p2")],
                },
                PromiseControlArgumentCallFacts::Invalid {
                    call_id: ToolCallId::new("detach-invalid"),
                },
                PromiseControlArgumentCallFacts::Invalid {
                    call_id: ToolCallId::new("missing"),
                },
            ]
        );
    }
}

pub(super) async fn invoke_batch(
    deps: &ToolActivityDeps,
    request: ToolInvokeBatchActivityRequest,
) -> Result<ToolBatchOutcome, ActivityError> {
    let request = request.request;
    match deps.tools.invoke_batch(request.clone()).await {
        Ok(result) => Ok(result),
        Err(error) => failed_tool_batch_result(deps.blobs.as_ref(), &request, error.to_string())
            .await
            .map(ToolBatchOutcome::completed)
            .map_err(activity_error),
    }
}
