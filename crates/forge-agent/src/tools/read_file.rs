use forge_llm::ToolDefinition;
use serde_json::json;
use std::sync::Arc;

use super::{READ_FILE_TOOL, RegisteredTool, required_string_argument};

pub(super) fn read_file_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: READ_FILE_TOOL.to_string(),
            description: "Read a file from the filesystem. Returns line-numbered content."
                .to_string(),
            parameters: json!({
                "type": "object",
                "required": ["file_path"],
                "properties": {
                    "file_path": { "type": "string" },
                    "offset": { "type": "integer" },
                    "limit": { "type": "integer" }
                },
                "additionalProperties": false
            }),
        },
        executor: Arc::new(|args, env| {
            Box::pin(async move {
                let file_path = required_string_argument(&args, "file_path")?;
                let offset = super::optional_usize_argument(&args, "offset")?;
                let limit = super::optional_usize_argument(&args, "limit")?;

                let content = env.read_file(&file_path, offset, limit).await?;
                Ok(super::format_line_numbered_content(
                    &content,
                    offset.unwrap_or(1),
                ))
            })
        }),
    }
}
