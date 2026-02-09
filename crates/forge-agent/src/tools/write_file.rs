use forge_llm::ToolDefinition;
use serde_json::json;
use std::sync::Arc;

use super::{RegisteredTool, WRITE_FILE_TOOL, required_string_argument};

pub(super) fn write_file_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: WRITE_FILE_TOOL.to_string(),
            description:
                "Write content to a file. Creates the file and parent directories if needed."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "required": ["file_path", "content"],
                "properties": {
                    "file_path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "additionalProperties": false
            }),
        },
        executor: Arc::new(|args, env| {
            Box::pin(async move {
                let file_path = required_string_argument(&args, "file_path")?;
                let content = required_string_argument(&args, "content")?;
                env.write_file(&file_path, &content).await?;
                Ok(format!("Wrote {} bytes to {}", content.len(), file_path))
            })
        }),
    }
}
