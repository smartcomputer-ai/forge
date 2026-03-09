//! End-to-end pipeline integration tests.
//!
//! These tests exercise the full stack: DOT file → forge-cli → real CLI agent
//! backend → JSONL event stream → disk artifacts → (optionally) CXDB persistence.
//!
//! They require CLI agent binaries installed at `~/.local/bin/` (claude, codex,
//! gemini) with OAuth authentication. No API keys needed.
//!
//! Run with: `cargo test -p forge-cli --test e2e_pipeline -- --ignored`

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const PIPELINE_TIMEOUT_SECS: u64 = 300; // 5 minutes per test

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

/// Create an isolated git repo inside the tempdir so CLI agents (especially
/// Codex) have a trusted working directory without polluting the real project.
fn init_sandbox(temp: &TempDir) {
    let status = Command::new("git")
        .args(["init", "--initial-branch=main"])
        .current_dir(temp.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("git init should succeed");
    assert!(status.success(), "git init failed in sandbox");

    // Codex requires at least one commit in the repo
    let status = Command::new("git")
        .args(["commit", "--allow-empty", "-m", "sandbox init"])
        .current_dir(temp.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("git commit should succeed");
    assert!(status.success(), "git commit failed in sandbox");
}

fn dot_file(name: &str) -> String {
    let path = examples_dir().join(name);
    assert!(
        path.exists(),
        "example DOT file not found at '{}'. Are you running from the workspace root?",
        path.display()
    );
    path.to_string_lossy().to_string()
}

// ---------------------------------------------------------------------------
// Binary resolution
// ---------------------------------------------------------------------------

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/home/ubuntu".to_string()))
}

fn resolve_bin(env_var: &str, default_name: &str) -> String {
    let path = std::env::var(env_var).unwrap_or_else(|_| {
        home_dir()
            .join(".local/bin")
            .join(default_name)
            .to_string_lossy()
            .to_string()
    });
    assert!(
        Path::new(&path).exists(),
        "CLI binary not found at '{path}'. Install it or set {env_var} to the correct path."
    );
    path
}

// ---------------------------------------------------------------------------
// CLI execution with timeout
// ---------------------------------------------------------------------------

fn run_pipeline(
    args: &[&str],
    cwd: &Path,
    env_vars: &[(&str, &str)],
    timeout_secs: u64,
) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_forge-cli"));
    cmd.args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env_vars {
        cmd.env(key, value);
    }

    let mut child = cmd.spawn().expect("forge-cli should start");

    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                return child.wait_with_output().expect("failed to collect output");
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "forge-cli timed out after {timeout_secs}s. The agent may be hanging. \
                         Check stderr for details."
                    );
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(e) => panic!("error waiting on forge-cli: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// JSONL event parsing
// ---------------------------------------------------------------------------

fn parse_events(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

fn events_by_category<'a>(events: &'a [Value], category: &str) -> Vec<&'a Value> {
    events
        .iter()
        .filter(|e| {
            e.get("kind")
                .and_then(|k| k.get("category"))
                .and_then(Value::as_str)
                == Some(category)
        })
        .collect()
}

fn events_by<'a>(events: &'a [Value], category: &str, kind: &str) -> Vec<&'a Value> {
    events
        .iter()
        .filter(|e| {
            let k = e.get("kind");
            k.and_then(|k| k.get("category"))
                .and_then(Value::as_str)
                == Some(category)
                && k.and_then(|k| k.get("kind"))
                    .and_then(Value::as_str)
                    == Some(kind)
        })
        .collect()
}

fn event_node_id(event: &Value) -> Option<&str> {
    event
        .get("kind")
        .and_then(|k| k.get("node_id"))
        .and_then(Value::as_str)
}

// ---------------------------------------------------------------------------
// Disk artifact assertions
// ---------------------------------------------------------------------------

fn assert_manifest(logs_root: &Path, expected_run_id: &str) {
    let manifest_path = logs_root.join("manifest.json");
    assert!(
        manifest_path.exists(),
        "manifest.json not found at {}",
        manifest_path.display()
    );
    let content = std::fs::read_to_string(&manifest_path).expect("read manifest.json");
    let manifest: Value = serde_json::from_str(&content).expect("parse manifest.json");
    assert_eq!(
        manifest.get("run_id").and_then(Value::as_str),
        Some(expected_run_id),
        "manifest.json run_id mismatch"
    );
}

/// Assert node completed successfully (outcome is "success" or "partial_success").
///
/// `partial_success` is valid — it means the agent completed with minor tool errors
/// but the pipeline engine treats it as a pass (same routing as "success").
fn assert_node_succeeded(logs_root: &Path, node_id: &str) {
    let status_path = logs_root.join(node_id).join("status.json");
    assert!(
        status_path.exists(),
        "status.json not found for node '{node_id}' at {}",
        status_path.display()
    );
    let content = std::fs::read_to_string(&status_path).expect("read status.json");
    let status: Value = serde_json::from_str(&content).expect("parse status.json");
    let outcome = status
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or("missing");
    assert!(
        outcome == "success" || outcome == "partial_success",
        "node '{node_id}' expected success/partial_success, got '{outcome}': {status}"
    );
}

fn assert_node_has_prompt(logs_root: &Path, node_id: &str, expected_substring: &str) {
    let prompt_path = logs_root.join(node_id).join("prompt.md");
    assert!(
        prompt_path.exists(),
        "prompt.md not found for node '{node_id}' at {}",
        prompt_path.display()
    );
    let content = std::fs::read_to_string(&prompt_path).expect("read prompt.md");
    assert!(
        content.contains(expected_substring),
        "node '{node_id}' prompt.md does not contain '{expected_substring}'. Got:\n{content}"
    );
}

fn assert_node_has_response(logs_root: &Path, node_id: &str) {
    let response_path = logs_root.join(node_id).join("response.md");
    assert!(
        response_path.exists(),
        "response.md not found for node '{node_id}' at {}",
        response_path.display()
    );
    let content = std::fs::read_to_string(&response_path).expect("read response.md");
    assert!(
        !content.trim().is_empty(),
        "node '{node_id}' response.md is empty"
    );
    assert!(
        !content.contains("[Simulated]"),
        "node '{node_id}' response.md contains mock output — expected real agent response"
    );
}

// ---------------------------------------------------------------------------
// Summary output assertions
// ---------------------------------------------------------------------------

fn assert_summary_status(stdout: &str, expected: &str) {
    assert!(
        stdout.contains(&format!("status: {expected}")),
        "expected 'status: {expected}' in output. Got:\n{stdout}"
    );
}

fn assert_summary_contains_nodes(stdout: &str, nodes: &[&str]) {
    for node in nodes {
        assert!(
            stdout.contains(node),
            "expected node '{node}' in completed_nodes output. Got:\n{stdout}"
        );
    }
}

// ---------------------------------------------------------------------------
// Shared test helpers
// ---------------------------------------------------------------------------

fn run_linear_pipeline(backend: &str, env_var: &str, bin_name: &str) {
    let _bin = resolve_bin(env_var, bin_name);

    let temp = TempDir::new().expect("tempdir");
    init_sandbox(&temp);
    let logs_root = temp.path().join("logs");
    let run_id = format!("e2e-linear-{backend}");

    let output = run_pipeline(
        &[
            "run",
            "--dot-file",
            &dot_file("01-linear-foundation.dot"),
            "--backend",
            backend,
            "--event-json",
            "--interviewer",
            "auto",
            "--logs-root",
            logs_root.to_str().unwrap(),
            "--run-id",
            &run_id,
        ],
        temp.path(),
        &[("FORGE_CXDB_PERSISTENCE", "off")],
        PIPELINE_TIMEOUT_SECS,
    );

    assert!(
        output.status.success(),
        "{backend} linear pipeline failed.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    let events = parse_events(&stdout);

    // Pipeline lifecycle events
    assert!(
        !events_by(&events, "pipeline", "started").is_empty(),
        "{backend}: missing pipeline.started event"
    );
    assert!(
        !events_by(&events, "pipeline", "completed").is_empty(),
        "{backend}: missing pipeline.completed event"
    );

    // Stage events for plan and summarize
    let stage_started = events_by(&events, "stage", "started");
    let stage_completed = events_by(&events, "stage", "completed");
    assert!(
        stage_started
            .iter()
            .any(|e| event_node_id(e) == Some("plan")),
        "{backend}: missing stage.started for 'plan'"
    );
    assert!(
        stage_completed
            .iter()
            .any(|e| event_node_id(e) == Some("plan")),
        "{backend}: missing stage.completed for 'plan'"
    );
    assert!(
        stage_started
            .iter()
            .any(|e| event_node_id(e) == Some("summarize")),
        "{backend}: missing stage.started for 'summarize'"
    );
    assert!(
        stage_completed
            .iter()
            .any(|e| event_node_id(e) == Some("summarize")),
        "{backend}: missing stage.completed for 'summarize'"
    );

    // Disk artifacts
    assert_manifest(&logs_root, &run_id);
    assert_node_succeeded(&logs_root, "plan");
    assert_node_has_prompt(&logs_root, "plan", "implementation plan");
    assert_node_has_response(&logs_root, "plan");
    assert_node_succeeded(&logs_root, "summarize");
    assert_node_has_response(&logs_root, "summarize");

    // Summary output
    assert_summary_status(&stdout, "success");
    assert_summary_contains_nodes(&stdout, &["plan", "summarize"]);
}

// ===========================================================================
// Test cases
// ===========================================================================

// ---------------------------------------------------------------------------
// 1-3. Linear pipeline × 3 backends
// ---------------------------------------------------------------------------

#[test]
#[ignore = "e2e: requires claude CLI (Tier 2, OAuth, slow)"]
fn e2e_linear_claude_code() {
    run_linear_pipeline("claude-code", "FORGE_CLAUDE_BIN", "claude");
}

#[test]
#[ignore = "e2e: requires codex CLI (Tier 2, OAuth, slow)"]
fn e2e_linear_codex() {
    run_linear_pipeline("codex-cli", "FORGE_CODEX_BIN", "codex");
}

#[test]
#[ignore = "e2e: requires gemini CLI (Tier 2, OAuth, slow)"]
fn e2e_linear_gemini() {
    run_linear_pipeline("gemini-cli", "FORGE_GEMINI_BIN", "gemini");
}

// ---------------------------------------------------------------------------
// 4. HITL pipeline with auto-approve
// ---------------------------------------------------------------------------

#[test]
#[ignore = "e2e: requires claude CLI (Tier 2, OAuth, slow)"]
fn e2e_hitl_auto_approve() {
    let _bin = resolve_bin("FORGE_CLAUDE_BIN", "claude");

    let temp = TempDir::new().expect("tempdir");
    init_sandbox(&temp);
    let logs_root = temp.path().join("logs");
    let run_id = "e2e-hitl";

    let output = run_pipeline(
        &[
            "run",
            "--dot-file",
            &dot_file("02-hitl-review-gate.dot"),
            "--backend",
            "claude-code",
            "--event-json",
            "--interviewer",
            "auto",
            "--logs-root",
            logs_root.to_str().unwrap(),
            "--run-id",
            run_id,
        ],
        temp.path(),
        &[("FORGE_CXDB_PERSISTENCE", "off")],
        PIPELINE_TIMEOUT_SECS,
    );

    assert!(
        output.status.success(),
        "HITL pipeline failed.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    let events = parse_events(&stdout);

    // Interview events present
    let interview_events = events_by_category(&events, "interview");
    assert!(
        !interview_events.is_empty(),
        "expected interview events for the HITL gate"
    );
    assert!(
        !events_by(&events, "interview", "started").is_empty(),
        "missing interview.started event"
    );
    assert!(
        !events_by(&events, "interview", "completed").is_empty(),
        "missing interview.completed event"
    );

    // Auto-approved path: implement → review_gate → ship → exit
    assert_summary_status(&stdout, "success");
    assert_summary_contains_nodes(&stdout, &["implement", "ship"]);

    // Artifacts for implement node
    assert_node_has_response(&logs_root, "implement");
}

// ---------------------------------------------------------------------------
// 5. Parallel pipeline
// ---------------------------------------------------------------------------

#[test]
#[ignore = "e2e: requires claude CLI (Tier 2, OAuth, slow)"]
fn e2e_parallel_pipeline() {
    let _bin = resolve_bin("FORGE_CLAUDE_BIN", "claude");

    let temp = TempDir::new().expect("tempdir");
    init_sandbox(&temp);
    let logs_root = temp.path().join("logs");
    let run_id = "e2e-parallel";

    let output = run_pipeline(
        &[
            "run",
            "--dot-file",
            &dot_file("03-parallel-triage-and-fanin.dot"),
            "--backend",
            "claude-code",
            "--event-json",
            "--interviewer",
            "auto",
            "--logs-root",
            logs_root.to_str().unwrap(),
            "--run-id",
            run_id,
        ],
        temp.path(),
        &[("FORGE_CXDB_PERSISTENCE", "off")],
        PIPELINE_TIMEOUT_SECS,
    );

    assert!(
        output.status.success(),
        "parallel pipeline failed.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    let events = parse_events(&stdout);

    // Parallel lifecycle events
    let par_started = events_by(&events, "parallel", "started");
    assert!(
        !par_started.is_empty(),
        "missing parallel.started event"
    );

    let branch_started = events_by(&events, "parallel", "branch_started");
    assert_eq!(
        branch_started.len(),
        3,
        "expected 3 parallel.branch_started events, got {}",
        branch_started.len()
    );

    let branch_completed = events_by(&events, "parallel", "branch_completed");
    assert_eq!(
        branch_completed.len(),
        3,
        "expected 3 parallel.branch_completed events, got {}",
        branch_completed.len()
    );

    let par_completed = events_by(&events, "parallel", "completed");
    assert!(
        !par_completed.is_empty(),
        "missing parallel.completed event"
    );

    // Interview event for decision gate
    assert!(
        !events_by_category(&events, "interview").is_empty(),
        "expected interview event for decision gate"
    );

    // All 3 branches should have completed
    assert_summary_status(&stdout, "success");
    assert_summary_contains_nodes(
        &stdout,
        &["investigate_logs", "inspect_tests", "review_recent_changes"],
    );
}

// ---------------------------------------------------------------------------
// 6. CXDB persistence
// ---------------------------------------------------------------------------

#[test]
#[ignore = "e2e: requires claude CLI + running CXDB server (Tier 2)"]
fn e2e_cxdb_persistence() {
    use forge_cxdb_runtime::{CxdbReqwestHttpClient, CxdbRuntimeStore, CxdbSdkBinaryClient};
    use std::sync::Arc;

    let _bin = resolve_bin("FORGE_CLAUDE_BIN", "claude");

    let temp = TempDir::new().expect("tempdir");
    init_sandbox(&temp);
    let logs_root = temp.path().join("logs");
    let run_id = "e2e-cxdb-persist";

    let output = run_pipeline(
        &[
            "run",
            "--dot-file",
            &dot_file("01-linear-foundation.dot"),
            "--backend",
            "claude-code",
            "--event-json",
            "--interviewer",
            "auto",
            "--logs-root",
            logs_root.to_str().unwrap(),
            "--run-id",
            run_id,
        ],
        temp.path(),
        &[("FORGE_CXDB_PERSISTENCE", "required")],
        PIPELINE_TIMEOUT_SECS,
    );

    assert!(
        output.status.success(),
        "CXDB pipeline failed.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    assert_summary_status(&stdout, "success");

    // Read cxdb_context_id from manifest.json
    let manifest_path = logs_root.join("manifest.json");
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path).expect("read manifest.json"),
    )
    .expect("parse manifest.json");
    let context_id = manifest
        .get("cxdb_context_id")
        .and_then(Value::as_str)
        .expect("manifest.json should contain cxdb_context_id when persistence=required");

    // Connect to CXDB and verify records
    let binary_addr = std::env::var("FORGE_CXDB_BINARY_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:9009".to_string());
    let http_base_url = std::env::var("FORGE_CXDB_HTTP_BASE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:9010".to_string());

    let binary_client = Arc::new(
        CxdbSdkBinaryClient::connect(&binary_addr)
            .expect("CXDB binary client should connect — is CXDB running?"),
    );
    let http_client = Arc::new(CxdbReqwestHttpClient::new(http_base_url));
    let store = CxdbRuntimeStore::new(binary_client, http_client);

    // Query run lifecycle records
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    rt.block_on(async {
        use forge_attractor::storage::types::{
            RunLifecycleRecord, StageLifecycleRecord,
            ATTRACTOR_RUN_LIFECYCLE_TYPE_ID, ATTRACTOR_STAGE_LIFECYCLE_TYPE_ID,
        };

        // List all turns and filter by type_id before decoding, since the
        // context contains mixed record types (run, stage, dot_source, etc.)
        let all_turns = store
            .list_turns(&context_id.to_string(), None, 200)
            .await
            .expect("list turns from CXDB");

        assert!(
            !all_turns.is_empty(),
            "no turns found in CXDB context {context_id}"
        );

        // Filter + decode run lifecycle records
        let run_turns: Vec<_> = all_turns
            .iter()
            .filter(|t| t.type_id == ATTRACTOR_RUN_LIFECYCLE_TYPE_ID)
            .collect();
        assert!(
            !run_turns.is_empty(),
            "no run lifecycle records found. Turn types: {:?}",
            all_turns.iter().map(|t| &t.type_id).collect::<Vec<_>>()
        );

        let run_records: Vec<RunLifecycleRecord> = run_turns
            .iter()
            .map(|t| {
                CxdbRuntimeStore::<
                    Arc<CxdbSdkBinaryClient>,
                    Arc<CxdbReqwestHttpClient>,
                >::decode_typed_payload(&t.payload)
                .unwrap_or_else(|e| panic!("decode run record: {e}"))
            })
            .collect();

        let run_kinds: Vec<&str> = run_records.iter().map(|r| r.kind.as_str()).collect();
        assert!(
            run_kinds.contains(&"initialized"),
            "missing 'initialized' run lifecycle record. Got: {run_kinds:?}"
        );
        assert!(
            run_kinds.contains(&"finalized"),
            "missing 'finalized' run lifecycle record. Got: {run_kinds:?}"
        );

        for record in &run_records {
            assert_eq!(record.run_id, run_id, "run_id mismatch in CXDB record");
        }

        // Filter + decode stage lifecycle records
        let stage_turns: Vec<_> = all_turns
            .iter()
            .filter(|t| t.type_id == ATTRACTOR_STAGE_LIFECYCLE_TYPE_ID)
            .collect();

        let stage_records: Vec<StageLifecycleRecord> = stage_turns
            .iter()
            .map(|t| {
                CxdbRuntimeStore::<
                    Arc<CxdbSdkBinaryClient>,
                    Arc<CxdbReqwestHttpClient>,
                >::decode_typed_payload(&t.payload)
                .unwrap_or_else(|e| panic!("decode stage record: {e}"))
            })
            .collect();

        let stage_node_ids: Vec<&str> = stage_records.iter().map(|r| r.node_id.as_str()).collect();
        assert!(
            stage_node_ids.contains(&"plan"),
            "missing stage records for 'plan' in CXDB. Got: {stage_node_ids:?}"
        );
        assert!(
            stage_node_ids.contains(&"summarize"),
            "missing stage records for 'summarize' in CXDB. Got: {stage_node_ids:?}"
        );
    });
}

// ---------------------------------------------------------------------------
// 7. Cross-provider structural parity
// ---------------------------------------------------------------------------

#[test]
#[ignore = "e2e: requires all 3 CLI agents (Tier 2, OAuth, very slow)"]
fn e2e_cross_provider_parity() {
    let backends = [
        ("claude-code", "FORGE_CLAUDE_BIN", "claude"),
        ("codex-cli", "FORGE_CODEX_BIN", "codex"),
        ("gemini-cli", "FORGE_GEMINI_BIN", "gemini"),
    ];

    struct RunResult {
        backend: String,
        event_categories: Vec<String>,
        completed_nodes: Vec<String>,
        has_plan_status: bool,
        has_plan_prompt: bool,
        has_plan_response: bool,
        has_summarize_status: bool,
    }

    let mut results: Vec<RunResult> = Vec::new();

    for (backend, env_var, bin_name) in &backends {
        let _bin = resolve_bin(env_var, bin_name);
        let temp = TempDir::new().expect("tempdir");
        init_sandbox(&temp);
        let logs_root = temp.path().join("logs");
        let run_id = format!("e2e-parity-{backend}");

        let output = run_pipeline(
            &[
                "run",
                "--dot-file",
                &dot_file("01-linear-foundation.dot"),
                "--backend",
                backend,
                "--event-json",
                "--interviewer",
                "auto",
                "--logs-root",
                logs_root.to_str().unwrap(),
                "--run-id",
                &run_id,
            ],
            temp.path(),
            &[("FORGE_CXDB_PERSISTENCE", "off")],
            PIPELINE_TIMEOUT_SECS,
        );

        assert!(
            output.status.success(),
            "{backend} parity test failed.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
        let events = parse_events(&stdout);

        let mut categories: Vec<String> = events
            .iter()
            .filter_map(|e| {
                e.get("kind")
                    .and_then(|k| k.get("category"))
                    .and_then(Value::as_str)
                    .map(String::from)
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        categories.sort();

        // Extract completed_nodes from summary
        let completed: Vec<String> = stdout
            .lines()
            .find(|line| line.starts_with("completed_nodes:"))
            .map(|line| {
                line.trim_start_matches("completed_nodes:")
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        results.push(RunResult {
            backend: backend.to_string(),
            event_categories: categories,
            completed_nodes: completed,
            has_plan_status: logs_root.join("plan").join("status.json").exists(),
            has_plan_prompt: logs_root.join("plan").join("prompt.md").exists(),
            has_plan_response: logs_root.join("plan").join("response.md").exists(),
            has_summarize_status: logs_root.join("summarize").join("status.json").exists(),
        });
    }

    // All backends should produce the same structural results
    let ref_result = &results[0];
    for result in &results[1..] {
        assert_eq!(
            result.event_categories, ref_result.event_categories,
            "event categories differ: {} has {:?}, {} has {:?}",
            result.backend, result.event_categories, ref_result.backend, ref_result.event_categories
        );
        assert_eq!(
            result.completed_nodes, ref_result.completed_nodes,
            "completed nodes differ: {} has {:?}, {} has {:?}",
            result.backend, result.completed_nodes, ref_result.backend, ref_result.completed_nodes
        );
        assert!(result.has_plan_status, "{}: missing plan/status.json", result.backend);
        assert!(result.has_plan_prompt, "{}: missing plan/prompt.md", result.backend);
        assert!(result.has_plan_response, "{}: missing plan/response.md", result.backend);
        assert!(result.has_summarize_status, "{}: missing summarize/status.json", result.backend);
    }
}

// ---------------------------------------------------------------------------
// 8. Resume from checkpoint
// ---------------------------------------------------------------------------

#[test]
#[ignore = "e2e: requires claude CLI (Tier 2, OAuth, slow)"]
fn e2e_resume_from_checkpoint() {
    let _bin = resolve_bin("FORGE_CLAUDE_BIN", "claude");

    let temp = TempDir::new().expect("tempdir");
    init_sandbox(&temp);
    let logs_root = temp.path().join("logs");
    let run_id = "e2e-resume";

    // First: run the pipeline to produce a checkpoint
    let output = run_pipeline(
        &[
            "run",
            "--dot-file",
            &dot_file("01-linear-foundation.dot"),
            "--backend",
            "claude-code",
            "--event-json",
            "--interviewer",
            "auto",
            "--logs-root",
            logs_root.to_str().unwrap(),
            "--run-id",
            run_id,
        ],
        temp.path(),
        &[("FORGE_CXDB_PERSISTENCE", "off")],
        PIPELINE_TIMEOUT_SECS,
    );

    assert!(
        output.status.success(),
        "initial run failed.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify checkpoint was written
    let checkpoint_path = logs_root.join("checkpoint.json");
    assert!(
        checkpoint_path.exists(),
        "checkpoint.json not found after initial run"
    );

    // Second: resume from the checkpoint
    let resume_logs = temp.path().join("resume_logs");
    let resume_output = run_pipeline(
        &[
            "resume",
            "--dot-file",
            &dot_file("01-linear-foundation.dot"),
            "--checkpoint",
            checkpoint_path.to_str().unwrap(),
            "--backend",
            "claude-code",
            "--event-json",
            "--interviewer",
            "auto",
            "--logs-root",
            resume_logs.to_str().unwrap(),
            "--run-id",
            run_id,
        ],
        temp.path(),
        &[("FORGE_CXDB_PERSISTENCE", "off")],
        PIPELINE_TIMEOUT_SECS,
    );

    assert!(
        resume_output.status.success(),
        "resume failed.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&resume_output.stdout),
        String::from_utf8_lossy(&resume_output.stderr)
    );

    let resume_stdout = String::from_utf8(resume_output.stdout).expect("stdout utf8");
    let resume_events = parse_events(&resume_stdout);

    // Should have pipeline.resumed (not pipeline.started)
    let resumed = events_by(&resume_events, "pipeline", "resumed");
    assert!(
        !resumed.is_empty(),
        "expected pipeline.resumed event on resume. Events: {:?}",
        resume_events
            .iter()
            .filter_map(|e| e.get("kind").and_then(|k| k.get("kind")).and_then(Value::as_str))
            .collect::<Vec<_>>()
    );

    assert_summary_status(&resume_stdout, "success");
}
