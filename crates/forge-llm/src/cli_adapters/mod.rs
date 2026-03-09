//! CLI agent providers that spawn coding agent CLIs as subprocesses.
//!
//! Each adapter implements `AgentProvider` by spawning a CLI binary,
//! parsing its JSONL output, and returning the completed result.

pub mod claude_code;
pub mod codex;
pub mod gemini;
