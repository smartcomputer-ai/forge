use crate::GrepOptions;
use forge_llm::ToolDefinition;
use serde_json::json;
use std::sync::Arc;

use super::{
    GREP_TOOL, RegisteredTool, optional_bool_argument, optional_string_argument,
    optional_usize_argument, required_string_argument,
};

pub(super) fn grep_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: GREP_TOOL.to_string(),
            description: "Search file contents using regex patterns.".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["pattern"],
                "properties": {
                    "pattern": { "type": "string" },
                    "path": { "type": "string" },
                    "glob_filter": { "type": "string" },
                    "case_insensitive": { "type": "boolean" },
                    "max_results": { "type": "integer" }
                },
                "additionalProperties": false
            }),
        },
        executor: Arc::new(|args, env| {
            Box::pin(async move {
                let pattern = required_string_argument(&args, "pattern")?;
                let path = optional_string_argument(&args, "path")?.unwrap_or(".".to_string());
                let options = GrepOptions {
                    glob_filter: optional_string_argument(&args, "glob_filter")?,
                    case_insensitive: optional_bool_argument(&args, "case_insensitive")?
                        .unwrap_or(false),
                    max_results: optional_usize_argument(&args, "max_results")?.or(Some(100)),
                };

                let output = env.grep(&pattern, &path, options).await?;
                if output.trim().is_empty() {
                    Ok("No matches found".to_string())
                } else {
                    Ok(output)
                }
            })
        }),
    }
}
