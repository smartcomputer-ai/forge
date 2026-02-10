use forge_attractor::{
    CheckpointMetadata, CheckpointNodeOutcome, CheckpointState, RuntimeContext, parse_dot,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn write_dot_file(path: &Path) {
    let source = r#"
        digraph G {
            start [shape=Mdiamond]
            plan [shape=box]
            exit [shape=Msquare]
            start -> plan -> exit
        }
    "#;
    std::fs::write(path, source).expect("dot file write should succeed");
}

fn write_hitl_dot_file(path: &Path) {
    let source = r#"
        digraph G {
            start [shape=Mdiamond]
            gate [shape=hexagon, label="Review"]
            yes
            no
            exit [shape=Msquare]
            start -> gate
            gate -> yes [label="[Y] Yes"]
            gate -> no [label="[N] No"]
            yes -> exit
            no -> exit
        }
    "#;
    std::fs::write(path, source).expect("dot file write should succeed");
}

fn write_resume_checkpoint(path: &Path) {
    let graph = parse_dot(
        r#"
        digraph G {
            start [shape=Mdiamond]
            plan [shape=box]
            exit [shape=Msquare]
            start -> plan -> exit
        }
        "#,
    )
    .expect("graph should parse");
    let start = graph.nodes.get("start").expect("start node should exist");

    let checkpoint = CheckpointState {
        metadata: CheckpointMetadata {
            schema_version: 1,
            run_id: "G-run".to_string(),
            checkpoint_id: "cp-1".to_string(),
            sequence_no: 1,
            timestamp: "1.000Z".to_string(),
        },
        current_node: "start".to_string(),
        next_node: Some("plan".to_string()),
        completed_nodes: vec!["start".to_string()],
        node_retries: BTreeMap::from([("start".to_string(), 0)]),
        node_outcomes: BTreeMap::from([(
            "start".to_string(),
            CheckpointNodeOutcome::from_runtime(&forge_attractor::NodeOutcome::success()),
        )]),
        context_values: RuntimeContext::new(),
        logs: vec![format!("checkpointed at {}", start.id)],
        current_node_fidelity: Some("compact".to_string()),
        terminal_status: None,
        terminal_failure_reason: None,
        graph_dot_source_hash: None,
        graph_dot_source_ref: None,
        graph_snapshot_hash: None,
        graph_snapshot_ref: None,
    };

    checkpoint
        .save_to_path(path)
        .expect("checkpoint should save");
}

fn run_cli(args: &[&str], cwd: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_forge-cli"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("cli process should start")
}

#[test]
fn run_command_dot_file_event_json_expected_success_output() {
    let temp = TempDir::new().expect("tempdir should create");
    let dot_file = temp.path().join("pipeline.dot");
    write_dot_file(&dot_file);

    let output = run_cli(
        &[
            "run",
            "--dot-file",
            dot_file.to_str().expect("dot file path should be utf8"),
            "--backend",
            "mock",
            "--event-json",
            "--interviewer",
            "auto",
        ],
        temp.path(),
    );

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("\"category\":\"pipeline\""));
    assert!(stdout.contains("run_id: G-run"));
    assert!(stdout.contains("status: success"));
}

#[test]
fn resume_command_checkpoint_expected_success_output() {
    let temp = TempDir::new().expect("tempdir should create");
    let dot_file = temp.path().join("pipeline.dot");
    let checkpoint_path = temp.path().join("checkpoint.json");
    write_dot_file(&dot_file);
    write_resume_checkpoint(&checkpoint_path);

    let output = run_cli(
        &[
            "resume",
            "--dot-file",
            dot_file.to_str().expect("dot file path should be utf8"),
            "--checkpoint",
            checkpoint_path
                .to_str()
                .expect("checkpoint path should be utf8"),
            "--backend",
            "mock",
            "--no-stream-events",
            "--interviewer",
            "auto",
        ],
        temp.path(),
    );

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("run_id: G-run"));
    assert!(stdout.contains("status: success"));
    assert!(stdout.contains("completed_nodes: start, plan"));
}

#[test]
fn inspect_checkpoint_json_expected_metadata_fields() {
    let temp = TempDir::new().expect("tempdir should create");
    let checkpoint_path = temp.path().join("checkpoint.json");
    write_resume_checkpoint(&checkpoint_path);

    let output = run_cli(
        &[
            "inspect-checkpoint",
            "--checkpoint",
            checkpoint_path
                .to_str()
                .expect("checkpoint path should be utf8"),
            "--json",
        ],
        temp.path(),
    );

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    let value: Value = serde_json::from_str(&stdout).expect("json output should parse");
    assert_eq!(
        value
            .get("metadata")
            .and_then(|metadata| metadata.get("run_id"))
            .and_then(Value::as_str),
        Some("G-run")
    );
    assert_eq!(
        value
            .get("next_node")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        Some("plan".to_string())
    );
}

#[test]
fn run_command_queue_interviewer_expected_human_answer_branch_selected() {
    let temp = TempDir::new().expect("tempdir should create");
    let dot_file = temp.path().join("pipeline.dot");
    write_hitl_dot_file(&dot_file);

    let output = run_cli(
        &[
            "run",
            "--dot-file",
            dot_file.to_str().expect("dot file path should be utf8"),
            "--backend",
            "mock",
            "--interviewer",
            "queue",
            "--human-answer",
            "N",
            "--no-stream-events",
        ],
        temp.path(),
    );

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("status: success"));
    assert!(stdout.contains("completed_nodes: start, gate, no"));
}
