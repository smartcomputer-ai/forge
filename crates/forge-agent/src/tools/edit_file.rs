use forge_llm::ToolDefinition;
use serde_json::json;
use std::sync::Arc;

use crate::{ToolError, patch};

use super::{EDIT_FILE_TOOL, RegisteredTool, optional_bool_argument, required_string_argument};

pub(super) fn edit_file_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: EDIT_FILE_TOOL.to_string(),
            description: "Replace an exact string occurrence in a file.".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["file_path", "old_string", "new_string"],
                "properties": {
                    "file_path": { "type": "string" },
                    "old_string": { "type": "string" },
                    "new_string": { "type": "string" },
                    "replace_all": { "type": "boolean" }
                },
                "additionalProperties": false
            }),
        },
        executor: Arc::new(|args, env| {
            Box::pin(async move {
                let file_path = required_string_argument(&args, "file_path")?;
                let old_string = required_string_argument(&args, "old_string")?;
                let new_string = required_string_argument(&args, "new_string")?;
                let replace_all = optional_bool_argument(&args, "replace_all")?.unwrap_or(false);
                if old_string.is_empty() {
                    return Err(
                        ToolError::Execution("old_string must not be empty".to_string()).into(),
                    );
                }

                let content = env.read_file(&file_path, None, None).await?;
                let (next_content, replacement_count) =
                    patch::apply_edit(&content, &file_path, &old_string, &new_string, replace_all)?;
                env.write_file(&file_path, &next_content).await?;

                Ok(format!(
                    "Updated {} ({} replacement{})",
                    file_path,
                    replacement_count,
                    if replacement_count == 1 { "" } else { "s" }
                ))
            })
        }),
    }
}
