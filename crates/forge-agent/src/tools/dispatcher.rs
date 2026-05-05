//! Tool dispatcher and handler registry.

use crate::effects::{ToolInvocationReceipt, ToolInvocationRequest};
use crate::refs::ArtifactRef;
use crate::tooling::{ToolExecutorKind, ToolRegistry, ToolRuntimeContext, ToolSpec};
use crate::tools::artifacts::{ArtifactStoreError, InMemoryToolArtifactStore, ToolArtifactStore};
use crate::tools::handler::{ToolExecutionError, ToolHandler, ToolInvocationContext};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ToolDispatcherError {
    #[error("duplicate handler id: {handler_id}")]
    DuplicateHandler { handler_id: String },

    #[error("duplicate tool handler binding for tool id: {tool_id}")]
    DuplicateToolBinding { tool_id: String },

    #[error("handler '{handler_id}' failed outside model-visible tool semantics: {detail}")]
    HandlerSystemFailure { handler_id: String, detail: String },

    #[error("artifact store error: {0}")]
    ArtifactStore(#[from] ArtifactStoreError),
}

#[derive(Default)]
pub struct ToolDispatcherBuilder {
    tool_registry: ToolRegistry,
    handlers: BTreeMap<String, Arc<dyn ToolHandler>>,
    tool_handler_bindings: BTreeMap<String, String>,
    artifacts: Option<Arc<dyn ToolArtifactStore>>,
}

impl ToolDispatcherBuilder {
    pub fn new(tool_registry: ToolRegistry) -> Self {
        Self {
            tool_registry,
            handlers: BTreeMap::new(),
            tool_handler_bindings: BTreeMap::new(),
            artifacts: None,
        }
    }

    pub fn with_artifacts(mut self, artifacts: Arc<dyn ToolArtifactStore>) -> Self {
        self.artifacts = Some(artifacts);
        self
    }

    pub fn register_handler(
        mut self,
        handler_id: impl Into<String>,
        handler: Arc<dyn ToolHandler>,
    ) -> Result<Self, ToolDispatcherError> {
        let handler_id = handler_id.into();
        if self.handlers.contains_key(&handler_id) {
            return Err(ToolDispatcherError::DuplicateHandler { handler_id });
        }
        self.handlers.insert(handler_id, handler);
        Ok(self)
    }

    pub fn bind_tool_handler(
        mut self,
        tool_id: impl Into<String>,
        handler_id: impl Into<String>,
    ) -> Result<Self, ToolDispatcherError> {
        let tool_id = tool_id.into();
        if self.tool_handler_bindings.contains_key(&tool_id) {
            return Err(ToolDispatcherError::DuplicateToolBinding { tool_id });
        }
        self.tool_handler_bindings
            .insert(tool_id, handler_id.into());
        Ok(self)
    }

    pub fn build(self) -> ToolDispatcher {
        ToolDispatcher {
            tool_registry: self.tool_registry,
            handlers: self.handlers,
            tool_handler_bindings: self.tool_handler_bindings,
            artifacts: self
                .artifacts
                .unwrap_or_else(|| Arc::new(InMemoryToolArtifactStore::new())),
        }
    }
}

#[derive(Clone)]
pub struct ToolDispatcher {
    tool_registry: ToolRegistry,
    handlers: BTreeMap<String, Arc<dyn ToolHandler>>,
    tool_handler_bindings: BTreeMap<String, String>,
    artifacts: Arc<dyn ToolArtifactStore>,
}

impl ToolDispatcher {
    pub fn builder(tool_registry: ToolRegistry) -> ToolDispatcherBuilder {
        ToolDispatcherBuilder::new(tool_registry)
    }

    pub fn tool_registry(&self) -> &ToolRegistry {
        &self.tool_registry
    }

    pub async fn dispatch(
        &self,
        request: ToolInvocationRequest,
        runtime: ToolRuntimeContext,
    ) -> Result<ToolInvocationReceipt, ToolDispatcherError> {
        let Some(tool) = self.resolve_tool(&request) else {
            return Ok(failed_receipt(
                &request,
                "unknown_tool",
                "unknown or unavailable tool",
            ));
        };
        if let Some(receipt) = validate_capabilities(&request, tool, &runtime) {
            return Ok(receipt);
        }
        if let Some(receipt) = self.validate_handler_binding(&request, tool) {
            return Ok(receipt);
        }

        let mut request = request;
        if let Some(receipt) = self.normalize_arguments(&mut request, tool).await? {
            return Ok(receipt);
        }

        let Some(handler_id) = self.resolve_handler_id(&request, tool) else {
            return Ok(failed_receipt(
                &request,
                "missing_handler",
                "tool has no registered handler",
            ));
        };
        let Some(handler) = self.handlers.get(&handler_id) else {
            return Ok(failed_receipt(
                &request,
                "unknown_handler",
                format!("unknown handler '{handler_id}'"),
            ));
        };

        let context = ToolInvocationContext::new(runtime, self.artifacts.clone());
        match handler.invoke(request.clone(), context).await {
            Ok(receipt) => Ok(receipt),
            Err(error) if error.model_visible => Ok(handler_error_receipt(&request, error)),
            Err(error) => Err(self.handler_error_to_dispatch_error(&handler_id, error)),
        }
    }

    fn resolve_tool<'a>(&'a self, request: &ToolInvocationRequest) -> Option<&'a ToolSpec> {
        if let Some(tool_id) = request.tool_id.as_ref() {
            return self.tool_registry.tools_by_id.get(tool_id);
        }
        self.tool_registry.tool_by_model_name(&request.tool_name)
    }

    fn resolve_handler_id(
        &self,
        request: &ToolInvocationRequest,
        tool: &ToolSpec,
    ) -> Option<String> {
        request
            .handler_id
            .clone()
            .or_else(|| match &tool.executor {
                ToolExecutorKind::Handler { handler_id } => Some(handler_id.clone()),
                _ => None,
            })
            .or_else(|| self.tool_handler_bindings.get(&tool.tool_id).cloned())
    }

    fn validate_handler_binding(
        &self,
        request: &ToolInvocationRequest,
        tool: &ToolSpec,
    ) -> Option<ToolInvocationReceipt> {
        let Some(request_handler_id) = request.handler_id.as_ref() else {
            return None;
        };
        match &tool.executor {
            ToolExecutorKind::Handler { handler_id } if handler_id == request_handler_id => None,
            ToolExecutorKind::Handler { handler_id } => Some(failed_receipt(
                request,
                "invalid_executor_binding",
                format!(
                    "tool '{}' is bound to handler '{}' but request used '{}'",
                    tool.tool_id, handler_id, request_handler_id
                ),
            )),
            _ if self
                .tool_handler_bindings
                .get(&tool.tool_id)
                .is_some_and(|handler_id| handler_id == request_handler_id) =>
            {
                None
            }
            _ => Some(failed_receipt(
                request,
                "invalid_executor_binding",
                format!(
                    "tool '{}' is not configured for handler '{}'",
                    tool.tool_id, request_handler_id
                ),
            )),
        }
    }

    async fn normalize_arguments(
        &self,
        request: &mut ToolInvocationRequest,
        tool: &ToolSpec,
    ) -> Result<Option<ToolInvocationReceipt>, ToolDispatcherError> {
        if request.arguments_json.is_none()
            && let Some(arguments_ref) = request.arguments_ref.as_ref()
        {
            request.arguments_json = Some(self.artifacts.read_text(arguments_ref).await?);
        }

        let Some(arguments_json) = request.arguments_json.as_ref() else {
            return Ok(Some(failed_receipt(
                request,
                "missing_arguments",
                "tool call has neither inline arguments nor argument ref",
            )));
        };

        let arguments = match serde_json::from_str::<Value>(arguments_json) {
            Ok(arguments) => arguments,
            Err(error) => {
                return Ok(Some(failed_receipt(
                    request,
                    "invalid_json_arguments",
                    format!("tool arguments are not valid JSON: {error}"),
                )));
            }
        };

        Ok(validate_json_arguments(request, tool, &arguments))
    }

    fn handler_error_to_dispatch_error(
        &self,
        handler_id: &str,
        error: ToolExecutionError,
    ) -> ToolDispatcherError {
        ToolDispatcherError::HandlerSystemFailure {
            handler_id: handler_id.into(),
            detail: error.detail,
        }
    }
}

fn validate_capabilities(
    request: &ToolInvocationRequest,
    tool: &ToolSpec,
    runtime: &ToolRuntimeContext,
) -> Option<ToolInvocationReceipt> {
    let missing = tool
        .required_capabilities
        .iter()
        .filter(|capability| !runtime.active_capabilities.contains(*capability))
        .cloned()
        .collect::<Vec<_>>();
    (!missing.is_empty()).then(|| {
        failed_receipt(
            request,
            "missing_capability",
            format!("missing required capabilities: {}", missing.join(",")),
        )
    })
}

fn validate_json_arguments(
    request: &ToolInvocationRequest,
    tool: &ToolSpec,
    arguments: &Value,
) -> Option<ToolInvocationReceipt> {
    if let Some(expected_type) = tool.args_schema.get("type").and_then(Value::as_str)
        && !json_type_matches(expected_type, arguments)
    {
        return Some(failed_receipt(
            request,
            "invalid_arguments",
            format!("tool arguments must be JSON {expected_type}"),
        ));
    }
    if let Some(required) = tool.args_schema.get("required").and_then(Value::as_array) {
        let missing = required
            .iter()
            .filter_map(Value::as_str)
            .filter(|field| arguments.get(field).is_none())
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Some(failed_receipt(
                request,
                "invalid_arguments",
                format!("missing required argument fields: {}", missing.join(",")),
            ));
        }
    }
    None
}

fn json_type_matches(expected_type: &str, arguments: &Value) -> bool {
    match expected_type {
        "object" => arguments.is_object(),
        "array" => arguments.is_array(),
        "string" => arguments.is_string(),
        "number" => arguments.is_number(),
        "integer" => arguments.as_i64().is_some() || arguments.as_u64().is_some(),
        "boolean" => arguments.is_boolean(),
        "null" => arguments.is_null(),
        _ => true,
    }
}

pub fn failed_receipt(
    request: &ToolInvocationRequest,
    code: impl Into<String>,
    detail: impl Into<String>,
) -> ToolInvocationReceipt {
    let code = code.into();
    let detail = detail.into();
    let output_ref = synthetic_error_ref(&request.call_id.to_string(), &detail);
    let mut metadata = request.metadata.clone();
    metadata.insert("error_code".into(), code);
    metadata.insert("error_detail".into(), detail.clone());
    ToolInvocationReceipt {
        call_id: request.call_id.clone(),
        tool_id: request.tool_id.clone(),
        tool_name: request.tool_name.clone(),
        output_ref: Some(output_ref.clone()),
        model_visible_output_ref: Some(output_ref),
        is_error: true,
        metadata,
    }
}

fn handler_error_receipt(
    request: &ToolInvocationRequest,
    error: ToolExecutionError,
) -> ToolInvocationReceipt {
    let mut receipt = failed_receipt(request, error.code, error.detail);
    if let Some(output_ref) = error.output_ref {
        receipt.output_ref = Some(output_ref);
    }
    if let Some(model_visible_output_ref) = error.model_visible_output_ref {
        receipt.model_visible_output_ref = Some(model_visible_output_ref);
    }
    receipt.metadata.extend(error.metadata);
    receipt
}

fn synthetic_error_ref(call_id: &str, detail: &str) -> ArtifactRef {
    ArtifactRef::new(format!("forge://tool-error/{call_id}")).with_preview(detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ToolCallId;
    use crate::testing::tools::{
        BackgroundInterruptHandler, BackgroundPollHandler, BackgroundStartHandler, EchoToolHandler,
        StaticToolHandler,
    };
    use crate::tooling::{ToolExecutorKind, ToolParallelismHint};
    use crate::tools::{ToolExecutionError, ToolInvocationStatus};
    use serde_json::json;
    use std::collections::{BTreeMap, BTreeSet};

    fn request() -> ToolInvocationRequest {
        ToolInvocationRequest {
            call_id: ToolCallId::new("call-1"),
            provider_call_id: Some("provider-call-1".into()),
            tool_id: Some("echo".into()),
            tool_name: "echo".into(),
            arguments_json: Some(r#"{"text":"hi"}"#.into()),
            arguments_ref: None,
            handler_id: None,
            context_ref: None,
            metadata: BTreeMap::new(),
        }
    }

    fn echo_tool() -> ToolSpec {
        ToolSpec {
            tool_id: "echo".into(),
            tool_name: "echo".into(),
            description: "Echo".into(),
            args_schema: json!({
                "type": "object",
                "required": ["text"],
            }),
            mapper: Default::default(),
            executor: ToolExecutorKind::Handler {
                handler_id: "echo-handler".into(),
            },
            parallelism_hint: ToolParallelismHint {
                parallel_safe: true,
                resource_key: None,
            },
            required_capabilities: Vec::new(),
            definition_ref: None,
            estimated_tokens: None,
            metadata: BTreeMap::new(),
        }
    }

    fn registry(tool: ToolSpec) -> ToolRegistry {
        let mut registry = ToolRegistry::default();
        registry.insert_tool(tool);
        registry
    }

    fn dispatcher(tool: ToolSpec) -> ToolDispatcher {
        ToolDispatcher::builder(registry(tool))
            .register_handler("echo-handler", Arc::new(EchoToolHandler::default()))
            .expect("register handler")
            .build()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_custom_handler_success_returns_terminal_receipt() {
        let receipt = dispatcher(echo_tool())
            .dispatch(request(), ToolRuntimeContext::default())
            .await
            .expect("dispatch");

        assert!(!receipt.is_error);
        assert_eq!(receipt.call_id, ToolCallId::new("call-1"));
        assert_eq!(
            receipt
                .model_visible_output_ref
                .as_ref()
                .and_then(|artifact| artifact.preview.as_deref()),
            Some(r#"{"text":"hi"}"#)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_unknown_tool_returns_model_visible_failure_receipt() {
        let mut request = request();
        request.tool_id = Some("missing".into());
        request.tool_name = "missing".into();

        let receipt = dispatcher(echo_tool())
            .dispatch(request, ToolRuntimeContext::default())
            .await
            .expect("dispatch");

        assert!(receipt.is_error);
        assert_eq!(
            receipt.metadata.get("error_code").map(String::as_str),
            Some("unknown_tool")
        );
        assert_eq!(
            receipt
                .model_visible_output_ref
                .as_ref()
                .and_then(|artifact| artifact.preview.as_deref()),
            Some("unknown or unavailable tool")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_invalid_json_arguments_returns_failure_receipt() {
        let mut request = request();
        request.arguments_json = Some("{".into());

        let receipt = dispatcher(echo_tool())
            .dispatch(request, ToolRuntimeContext::default())
            .await
            .expect("dispatch");

        assert!(receipt.is_error);
        assert_eq!(
            receipt.metadata.get("error_code").map(String::as_str),
            Some("invalid_json_arguments")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_missing_required_field_returns_failure_receipt() {
        let mut request = request();
        request.arguments_json = Some(r#"{"other":"hi"}"#.into());

        let receipt = dispatcher(echo_tool())
            .dispatch(request, ToolRuntimeContext::default())
            .await
            .expect("dispatch");

        assert!(receipt.is_error);
        assert_eq!(
            receipt.metadata.get("error_code").map(String::as_str),
            Some("invalid_arguments")
        );
        assert!(
            receipt
                .metadata
                .get("error_detail")
                .is_some_and(|detail| detail.contains("text"))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_missing_capability_returns_failure_before_handler() {
        let mut tool = echo_tool();
        tool.required_capabilities = vec!["shell".into()];

        let receipt = dispatcher(tool)
            .dispatch(request(), ToolRuntimeContext::default())
            .await
            .expect("dispatch");

        assert!(receipt.is_error);
        assert_eq!(
            receipt.metadata.get("error_code").map(String::as_str),
            Some("missing_capability")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_invalid_executor_binding_returns_failure_receipt() {
        let mut request = request();
        request.handler_id = Some("other-handler".into());

        let receipt = dispatcher(echo_tool())
            .dispatch(request, ToolRuntimeContext::default())
            .await
            .expect("dispatch");

        assert!(receipt.is_error);
        assert_eq!(
            receipt.metadata.get("error_code").map(String::as_str),
            Some("invalid_executor_binding")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_present_capability_allows_handler() {
        let mut tool = echo_tool();
        tool.required_capabilities = vec!["shell".into()];
        let runtime = ToolRuntimeContext {
            active_capabilities: BTreeSet::from(["shell".into()]),
            ..Default::default()
        };

        let receipt = dispatcher(tool)
            .dispatch(request(), runtime)
            .await
            .expect("dispatch");

        assert!(!receipt.is_error);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_loads_arguments_from_ref_before_validation() {
        let artifacts = Arc::new(InMemoryToolArtifactStore::new());
        let arguments_ref = artifacts.insert_text("mem://args/1", r#"{"text":"from-ref"}"#);
        let mut request = request();
        request.arguments_json = None;
        request.arguments_ref = Some(arguments_ref);
        let dispatcher = ToolDispatcher::builder(registry(echo_tool()))
            .with_artifacts(artifacts)
            .register_handler("echo-handler", Arc::new(EchoToolHandler::default()))
            .expect("register handler")
            .build();

        let receipt = dispatcher
            .dispatch(request, ToolRuntimeContext::default())
            .await
            .expect("dispatch");

        assert!(!receipt.is_error);
        assert_eq!(
            receipt
                .model_visible_output_ref
                .as_ref()
                .and_then(|artifact| artifact.preview.as_deref()),
            Some(r#"{"text":"from-ref"}"#)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn model_visible_handler_error_becomes_failed_receipt() {
        let handler = StaticToolHandler {
            result: Err(ToolExecutionError::tool_failure("bad_input", "bad input")),
        };
        let dispatcher = ToolDispatcher::builder(registry(echo_tool()))
            .register_handler("echo-handler", Arc::new(handler))
            .expect("register handler")
            .build();

        let receipt = dispatcher
            .dispatch(request(), ToolRuntimeContext::default())
            .await
            .expect("dispatch");

        assert!(receipt.is_error);
        assert_eq!(
            receipt.metadata.get("error_code").map(String::as_str),
            Some("bad_input")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn system_handler_error_remains_dispatch_error() {
        let handler = StaticToolHandler {
            result: Err(ToolExecutionError::system_failure(
                "network_down",
                "network unavailable",
            )),
        };
        let dispatcher = ToolDispatcher::builder(registry(echo_tool()))
            .register_handler("echo-handler", Arc::new(handler))
            .expect("register handler")
            .build();

        let error = dispatcher
            .dispatch(request(), ToolRuntimeContext::default())
            .await
            .expect_err("system failure");

        assert!(matches!(
            error,
            ToolDispatcherError::HandlerSystemFailure { .. }
        ));
    }

    #[test]
    fn result_metadata_applies_background_handle_fields_to_receipt() {
        let mut receipt = failed_receipt(&request(), "running", "still running");
        let metadata = crate::tools::ToolResultMetadata::still_running(
            crate::tools::ToolRuntimeHandle {
                handle_id: "proc-1".into(),
                kind: "process".into(),
                continuation_tool_ids: vec!["poll".into(), "interrupt".into()],
                metadata: BTreeMap::new(),
            },
            crate::tools::ToolRuntimeSnapshot {
                output_snapshot_ref: Some(ArtifactRef::new("mem://snapshot/1")),
                observed_at_ms: Some(42),
            },
        );

        metadata.apply_to_receipt(&mut receipt);

        assert_eq!(
            receipt.metadata.get("tool_status").map(String::as_str),
            Some(ToolInvocationStatus::StillRunning.as_str())
        );
        assert_eq!(
            receipt
                .metadata
                .get("runtime_handle_id")
                .map(String::as_str),
            Some("proc-1")
        );
        assert_eq!(
            receipt
                .metadata
                .get("continuation_tool_ids")
                .map(String::as_str),
            Some("poll,interrupt")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn background_handlers_return_running_then_complete_receipts() {
        let mut registry = ToolRegistry::default();
        registry.insert_tool(ToolSpec {
            tool_id: "start".into(),
            tool_name: "start".into(),
            description: "Start background work".into(),
            args_schema: json!({"type":"object"}),
            executor: ToolExecutorKind::Handler {
                handler_id: "start-handler".into(),
            },
            ..Default::default()
        });
        registry.insert_tool(ToolSpec {
            tool_id: "poll".into(),
            tool_name: "poll".into(),
            description: "Poll background work".into(),
            args_schema: json!({"type":"object"}),
            executor: ToolExecutorKind::Handler {
                handler_id: "poll-handler".into(),
            },
            ..Default::default()
        });
        registry.insert_tool(ToolSpec {
            tool_id: "interrupt".into(),
            tool_name: "interrupt".into(),
            description: "Interrupt background work".into(),
            args_schema: json!({"type":"object"}),
            executor: ToolExecutorKind::Handler {
                handler_id: "interrupt-handler".into(),
            },
            ..Default::default()
        });
        let dispatcher = ToolDispatcher::builder(registry)
            .register_handler(
                "start-handler",
                Arc::new(BackgroundStartHandler {
                    handle_id: "job-1".into(),
                    poll_tool_id: "poll".into(),
                    snapshot_ref: ArtifactRef::new("mem://snapshot/1").with_preview("running"),
                }),
            )
            .expect("register start")
            .register_handler(
                "poll-handler",
                Arc::new(BackgroundPollHandler {
                    completed_ref: ArtifactRef::new("mem://complete/1").with_preview("done"),
                }),
            )
            .expect("register poll")
            .register_handler(
                "interrupt-handler",
                Arc::new(BackgroundInterruptHandler {
                    interrupted_ref: ArtifactRef::new("mem://interrupted/1")
                        .with_preview("interrupted"),
                }),
            )
            .expect("register interrupt")
            .build();

        let running = dispatcher
            .dispatch(
                ToolInvocationRequest {
                    call_id: ToolCallId::new("call-start"),
                    provider_call_id: None,
                    tool_id: Some("start".into()),
                    tool_name: "start".into(),
                    arguments_json: Some("{}".into()),
                    arguments_ref: None,
                    handler_id: None,
                    context_ref: None,
                    metadata: BTreeMap::new(),
                },
                ToolRuntimeContext::default(),
            )
            .await
            .expect("start dispatch");
        assert_eq!(
            running.metadata.get("tool_status").map(String::as_str),
            Some("still_running")
        );
        assert_eq!(
            running
                .metadata
                .get("runtime_handle_id")
                .map(String::as_str),
            Some("job-1")
        );

        let completed = dispatcher
            .dispatch(
                ToolInvocationRequest {
                    call_id: ToolCallId::new("call-poll"),
                    provider_call_id: None,
                    tool_id: Some("poll".into()),
                    tool_name: "poll".into(),
                    arguments_json: Some(r#"{"handle_id":"job-1"}"#.into()),
                    arguments_ref: None,
                    handler_id: None,
                    context_ref: None,
                    metadata: BTreeMap::new(),
                },
                ToolRuntimeContext::default(),
            )
            .await
            .expect("poll dispatch");

        assert!(!completed.is_error);
        assert_eq!(
            completed.metadata.get("tool_status").map(String::as_str),
            Some("complete")
        );

        let interrupted = dispatcher
            .dispatch(
                ToolInvocationRequest {
                    call_id: ToolCallId::new("call-interrupt"),
                    provider_call_id: None,
                    tool_id: Some("interrupt".into()),
                    tool_name: "interrupt".into(),
                    arguments_json: Some(r#"{"handle_id":"job-1"}"#.into()),
                    arguments_ref: None,
                    handler_id: None,
                    context_ref: None,
                    metadata: BTreeMap::new(),
                },
                ToolRuntimeContext::default(),
            )
            .await
            .expect("interrupt dispatch");

        assert!(interrupted.is_error);
        assert_eq!(
            interrupted.metadata.get("tool_status").map(String::as_str),
            Some("cancelled")
        );
    }
}
