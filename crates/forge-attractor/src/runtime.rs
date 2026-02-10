use crate::storage::AttractorArtifactWriter;
use crate::{AttractorError, Graph, Node, RuntimeContext, handlers};
use async_trait::async_trait;
use forge_cxdb_runtime::CxdbTurnId as TurnId;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeStatus {
    Success,
    PartialSuccess,
    Retry,
    Fail,
}

impl NodeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::PartialSuccess => "partial_success",
            Self::Retry => "retry",
            Self::Fail => "fail",
        }
    }

    pub fn is_success_like(self) -> bool {
        matches!(self, Self::Success | Self::PartialSuccess)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeOutcome {
    pub status: NodeStatus,
    pub notes: Option<String>,
    pub context_updates: RuntimeContext,
    pub preferred_label: Option<String>,
    pub suggested_next_ids: Vec<String>,
}

impl NodeOutcome {
    pub fn success() -> Self {
        Self {
            status: NodeStatus::Success,
            notes: None,
            context_updates: RuntimeContext::new(),
            preferred_label: None,
            suggested_next_ids: Vec::new(),
        }
    }

    pub fn failure(reason: impl Into<String>) -> Self {
        Self {
            status: NodeStatus::Fail,
            notes: Some(reason.into()),
            context_updates: RuntimeContext::new(),
            preferred_label: None,
            suggested_next_ids: Vec::new(),
        }
    }
}

#[async_trait]
pub trait NodeExecutor: Send + Sync {
    async fn execute(
        &self,
        node: &Node,
        context: &RuntimeContext,
        graph: &Graph,
    ) -> Result<NodeOutcome, AttractorError>;
}

#[derive(Debug, Default)]
pub struct NoopNodeExecutor;

#[async_trait]
impl NodeExecutor for NoopNodeExecutor {
    async fn execute(
        &self,
        _node: &Node,
        _context: &RuntimeContext,
        _graph: &Graph,
    ) -> Result<NodeOutcome, AttractorError> {
        Ok(NodeOutcome::success())
    }
}

#[derive(Clone)]
pub struct RunConfig {
    pub run_id: Option<String>,
    pub base_turn_id: Option<TurnId>,
    pub storage: Option<crate::storage::SharedAttractorStorageWriter>,
    pub artifacts: Option<Arc<dyn AttractorArtifactWriter>>,
    pub cxdb_persistence: CxdbPersistenceMode,
    pub events: crate::RuntimeEventSink,
    pub executor: Arc<dyn NodeExecutor>,
    pub retry_backoff: crate::RetryBackoffConfig,
    pub logs_root: Option<PathBuf>,
    pub resume_from_checkpoint: Option<PathBuf>,
    pub max_loop_restarts: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CxdbPersistenceMode {
    Off,
    Required,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            run_id: None,
            base_turn_id: None,
            storage: None,
            artifacts: None,
            cxdb_persistence: CxdbPersistenceMode::Off,
            events: crate::RuntimeEventSink::default(),
            executor: Arc::new(handlers::registry::RegistryNodeExecutor::new(
                handlers::core_registry(),
            )),
            retry_backoff: crate::RetryBackoffConfig::default(),
            logs_root: None,
            resume_from_checkpoint: None,
            max_loop_restarts: 16,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipelineStatus {
    Success,
    Fail,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PipelineRunResult {
    pub run_id: String,
    pub status: PipelineStatus,
    pub failure_reason: Option<String>,
    pub completed_nodes: Vec<String>,
    pub node_outcomes: BTreeMap<String, NodeOutcome>,
    pub context: RuntimeContext,
}
