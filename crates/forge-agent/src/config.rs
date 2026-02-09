use std::collections::HashMap;

/// Runtime configuration for a coding-agent session.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionConfig {
    pub max_turns: usize,
    pub max_tool_rounds_per_input: usize,
    pub default_command_timeout_ms: u64,
    pub max_command_timeout_ms: u64,
    pub reasoning_effort: Option<String>,
    pub system_prompt_override: Option<String>,
    pub tool_output_limits: HashMap<String, usize>,
    pub tool_line_limits: HashMap<String, usize>,
    pub enable_loop_detection: bool,
    pub loop_detection_window: usize,
    pub max_subagent_depth: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            max_turns: 0,
            max_tool_rounds_per_input: 200,
            default_command_timeout_ms: 10_000,
            max_command_timeout_ms: 600_000,
            reasoning_effort: None,
            system_prompt_override: None,
            tool_output_limits: default_tool_output_limits(),
            tool_line_limits: default_tool_line_limits(),
            enable_loop_detection: true,
            loop_detection_window: 10,
            max_subagent_depth: 1,
        }
    }
}

pub fn default_tool_output_limits() -> HashMap<String, usize> {
    HashMap::from([
        ("read_file".to_string(), 50_000),
        ("shell".to_string(), 30_000),
        ("grep".to_string(), 20_000),
        ("glob".to_string(), 20_000),
        ("edit_file".to_string(), 10_000),
        ("apply_patch".to_string(), 10_000),
        ("write_file".to_string(), 1_000),
        ("spawn_agent".to_string(), 20_000),
    ])
}

pub fn default_tool_line_limits() -> HashMap<String, usize> {
    HashMap::from([
        ("shell".to_string(), 256),
        ("grep".to_string(), 200),
        ("glob".to_string(), 500),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_config_defaults_match_spec_baseline() {
        let config = SessionConfig::default();
        assert_eq!(config.max_turns, 0);
        assert_eq!(config.max_tool_rounds_per_input, 200);
        assert_eq!(config.default_command_timeout_ms, 10_000);
        assert_eq!(config.max_command_timeout_ms, 600_000);
        assert_eq!(config.system_prompt_override, None);
        assert_eq!(config.loop_detection_window, 10);
        assert_eq!(config.max_subagent_depth, 1);
    }
}
