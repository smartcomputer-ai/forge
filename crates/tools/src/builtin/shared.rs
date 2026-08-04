//! Shared helpers for built-in tool surfaces and operations.

use serde_json::{Value, json};

use crate::{environment::process::ProcessOutput, error::ToolError};

/// Model-visible marker for a search that stopped at one of its bounds,
/// naming the bound and how to narrow the search.
pub(super) fn visible_with_search_stop(
    mut visible: String,
    stopped: Option<crate::fs::FsSearchStop>,
) -> String {
    let Some(stopped) = stopped else {
        return visible;
    };
    let note = match stopped {
        crate::fs::FsSearchStop::MatchLimit => {
            "[truncated: match limit reached — narrow the pattern or lower the limit]"
        }
        crate::fs::FsSearchStop::FileLimit => {
            "[truncated: file budget exhausted — narrow the path or add an include filter]"
        }
        crate::fs::FsSearchStop::ByteLimit => {
            "[truncated: byte budget exhausted — narrow the path or add an include filter]"
        }
        crate::fs::FsSearchStop::TimeLimit => {
            "[truncated: time budget exhausted — narrow the path or add an include filter]"
        }
    };
    if !visible.is_empty() {
        visible.push('\n');
    }
    visible.push_str(note);
    visible
}

pub(super) fn process_visible_output(output: &ProcessOutput) -> String {
    let stdout = output.stdout.text_lossy();
    let stderr = output.stderr.text_lossy();
    let mut visible = match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("{stdout}\n{stderr}"),
        (false, true) => stdout,
        (true, false) => stderr,
        (true, true) => format!("process status: {:?}", output.status),
    };
    if output.orphaned_descendants {
        visible.push_str(
            "\n[note: the command left background processes running after it exited; \
             the host terminated them — make the command wait for or stop its children]",
        );
    }
    visible
}

pub(super) fn object<const N: usize, const M: usize>(
    properties: [(&'static str, Value); N],
    required: [&'static str; M],
) -> Value {
    let properties = properties
        .into_iter()
        .map(|(name, schema)| (name.to_string(), schema))
        .collect::<serde_json::Map<_, _>>();
    let required = required.into_iter().collect::<Vec<_>>();
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

pub(super) fn string(description: &'static str) -> Value {
    json!({ "type": "string", "description": description })
}

pub(super) fn nullable_string(description: &'static str) -> Value {
    json!({ "type": ["string", "null"], "description": description })
}

pub(super) fn integer(description: &'static str) -> Value {
    json!({ "type": "integer", "minimum": 0, "description": description })
}

pub(super) fn nullable_integer(description: &'static str) -> Value {
    json!({ "anyOf": [integer(description), { "type": "null" }] })
}

pub(super) fn boolean(description: &'static str) -> Value {
    json!({ "type": "boolean", "description": description })
}

pub(super) fn optional_boolean(description: &'static str) -> Value {
    json!({ "type": ["boolean", "null"], "description": description })
}

pub(super) fn optional_enum<const N: usize>(
    description: &'static str,
    values: [&'static str; N],
) -> Value {
    let values = values.into_iter().collect::<Vec<_>>();
    json!({
        "anyOf": [
            { "type": "string", "enum": values },
            { "type": "null" }
        ],
        "description": description
    })
}

pub(super) fn array_of_strings(description: &'static str) -> Value {
    json!({
        "type": "array",
        "items": { "type": "string" },
        "description": description
    })
}

pub(super) fn string_map(description: &'static str) -> Value {
    json!({
        "type": "object",
        "additionalProperties": { "type": "string" },
        "description": description
    })
}

pub(crate) fn invalid_request(message: impl Into<String>) -> ToolError {
    ToolError::InvalidRequest {
        message: message.into(),
    }
}
