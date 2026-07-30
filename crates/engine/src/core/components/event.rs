use serde::{Deserialize, Serialize};

use crate::{
    ContextEvent, CoreAgentLifecycleEvent, EnvironmentEvent, PromiseEvent, RunEvent,
    ToolConfigEvent, ToolEvent, TurnEvent, WorkflowToolConfigEvent, WorkflowToolEvent,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreAgentEvent {
    Lifecycle(CoreAgentLifecycleEvent),
    Run(RunEvent),
    Turn(TurnEvent),
    Context(ContextEvent),
    Environment(EnvironmentEvent),
    ToolConfig(ToolConfigEvent),
    Tool(ToolEvent),
    Promise(PromiseEvent),
    WorkflowToolConfig(WorkflowToolConfigEvent),
    WorkflowTool(WorkflowToolEvent),
}
