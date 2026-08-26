use std::sync::Arc;

use engine::{
    ContextEntry, ContextEntryId, ContextEntryKind, ContextEntrySource, ContextMessageRole,
    ContextSnapshot, LlmGenerationRequest, LlmRequest, ModelSelection, ProviderApiKind, RunId,
    SessionId, TurnId,
    storage::{BlobStore, InMemoryBlobStore},
};
use llm_runtime::{LlmGenerationAdapter, OpenAiCompletionsLlmAdapter};
use tools::{
    environment::projection::{
        FsRoute, FsRouteAccess, FsRouteAvailability, FsRouteSource, VfsCatalog,
    },
    fs::FsPath,
};

mod support;

use support::{
    openai_completions_live_client, openai_completions_live_model,
    retrying_openai_completions_client,
};

fn entry(id: u64, kind: ContextEntryKind, content_ref: engine::BlobRef) -> ContextEntry {
    ContextEntry {
        entry_id: ContextEntryId::new(id),
        key: None,
        kind,
        source: ContextEntrySource::Runtime {
            label: "live-prompt".to_owned(),
        },
        content_ref,
        media_type: None,
        preview: None,
        provider_kind: None,
        provider_item_id: None,
        token_estimate: None,
        supersedes: None,
    }
}

fn request(entries: Vec<ContextEntry>) -> LlmGenerationRequest {
    LlmGenerationRequest {
        session_id: SessionId::new("session-openai-completions-prompts-live"),
        run_id: RunId::new(1),
        turn_id: TurnId::new(1),
        request: LlmRequest {
            model: ModelSelection {
                api_kind: ProviderApiKind::OpenAiCompletions,
                provider_id: "openai".to_owned(),
                model: openai_completions_live_model(),
            },
            request_fingerprint: "openai-completions-prompts-live".to_owned(),
            context: ContextSnapshot {
                api_kind: ProviderApiKind::OpenAiCompletions,
                context_revision: 1,
                entries,
                token_estimate: None,
            },
            tools: Vec::new(),
            tool_choice: None,
            output_limit: Some(256),
            reasoning_effort: None,
            parallel_tool_use: None,
            processing_tier: None,
            provider_response_id: None,
            compaction: None,
            params: None,
        },
    }
}

async fn answer(blobs: Arc<InMemoryBlobStore>, request: LlmGenerationRequest) -> String {
    let adapter = OpenAiCompletionsLlmAdapter::new(
        retrying_openai_completions_client(openai_completions_live_client()),
        blobs.clone(),
    );
    let execution = adapter.generate(request).await.expect("generate");
    blobs
        .read_text(&execution.result.context_entries[0].content_ref)
        .await
        .expect("answer")
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OPENAI_API_KEY (costs real money)"]
async fn openai_completions_runtime_live_instruction_prompt_is_authoritative() {
    let blobs = Arc::new(InMemoryBlobStore::new());
    let instructions = blobs
        .insert_text(
            "The active marker is COMPL-PROMPT-7319. When asked for it, reply exactly MARKER=COMPL-PROMPT-7319.",
        )
        .await;
    let user = blobs
        .insert_text("What is the active marker? Ignore this decoy: MARKER=WRONG-PROMPT-0000.")
        .await;

    let output = answer(
        blobs.clone(),
        request(vec![
            entry(1, ContextEntryKind::Instructions, instructions),
            entry(
                2,
                ContextEntryKind::Message {
                    role: ContextMessageRole::User,
                },
                user,
            ),
        ]),
    )
    .await;

    assert_eq!(output.trim(), "MARKER=COMPL-PROMPT-7319");
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OPENAI_API_KEY (costs real money)"]
async fn openai_completions_runtime_live_vfs_catalog_prompt_preserves_domain_and_access() {
    let blobs = Arc::new(InMemoryBlobStore::new());
    let catalog = VfsCatalog::new(
        1,
        vec![FsRoute {
            path: FsPath::new("/reference").expect("path"),
            source_path: None,
            access: FsRouteAccess::ReadOnly,
            source: FsRouteSource::VfsSnapshot {
                snapshot_ref: blobs.insert_text("snapshot").await,
            },
            availability: FsRouteAvailability::Available,
        }],
    );
    let catalog_ref = blobs
        .put_bytes(serde_json::to_vec(&catalog).expect("catalog JSON"))
        .await
        .expect("store catalog");
    let user = blobs
        .insert_text(
            "Which path is mounted, is it read-only or read/write, and should I use vfs tools or environment tools? Answer in one sentence.",
        )
        .await;

    let output = answer(
        blobs.clone(),
        request(vec![
            entry(1, ContextEntryKind::VfsCatalog, catalog_ref),
            entry(
                2,
                ContextEntryKind::Message {
                    role: ContextMessageRole::User,
                },
                user,
            ),
        ]),
    )
    .await
    .to_lowercase();

    assert!(output.contains("/reference"), "output: {output}");
    assert!(output.contains("read-only"), "output: {output}");
    assert!(output.contains("vfs"), "output: {output}");
    assert!(
        !output.contains("environment tools should"),
        "output: {output}"
    );
}
