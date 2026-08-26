//! Writes the committed workflow contract under
//! `crates/temporal-workflow/contract/`.
//!
//! Usage: `cargo run -p temporal-workflow --bin export-workflow-contract [output-dir]`

use std::{env, fs, path::PathBuf};

fn main() {
    let out_dir = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("contract"));
    fs::create_dir_all(&out_dir).expect("create output directory");

    let exported = temporal_workflow::workflow_contract::export();
    let artifacts = [
        ("workflow.schema.json", &exported.schema_bundle),
        ("workflow.json", &exported.manifest),
    ];
    for (name, value) in artifacts {
        let mut text = serde_json::to_string_pretty(value).expect("serialize artifact");
        text.push('\n');
        let path = out_dir.join(name);
        fs::write(&path, text).expect("write artifact");
        println!("wrote {}", path.display());
    }

    let reference_path = out_dir.join("workflow-contract.md");
    fs::write(&reference_path, exported.reference).expect("write workflow reference");
    println!("wrote {}", reference_path.display());
}
