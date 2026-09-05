# eval

Prompt-level eval harness for Lightspeed agent workflows.

## Commands

- `cargo run -p eval -- list`
- `cargo run -p eval -- case read-file`
- `cargo run -p eval -- all --runs 3`
- `cargo run -p eval -- --provider openai-completions all`
- `cargo run -p eval -- --provider anthropic all`

`case` and `all` execute live provider calls. OpenAI Responses and Chat Completions
runs require `OPENAI_API_KEY`; Anthropic Messages runs require `ANTHROPIC_API_KEY`.
Provider base-URL overrides and the existing provider-specific live-model
environment variables are honored. Cases may declare a `providers` allowlist
when a tool is intentionally absent from one provider-native surface.

Each attempt gets separate temporary VFS and active-environment filesystem
roots, the `test-support` runner harness, and an inline builtin tool executor.
Case setup and expectations use `files` for VFS and `environment_files` for the
environment domain. Assertions cover logical tool calls, tool output text,
final assistant text, and the file state of both domains.
