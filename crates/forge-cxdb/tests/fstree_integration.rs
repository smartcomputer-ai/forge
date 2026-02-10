// Copyright 2025 StrongDM Inc
// SPDX-License-Identifier: Apache-2.0

use cxdb::fstree;
use cxdb::types::{new_user_input, TypeIDConversationItem, TypeVersionConversationItem};
use cxdb::{dial, encode_msgpack, AppendRequest, RequestContext};
use serde_json::Value;

#[test]
fn integration_fstree_snapshot_http() {
    if std::env::var("CXDB_INTEGRATION").is_err() {
        eprintln!("CXDB_INTEGRATION not set; skipping integration test");
        return;
    }

    let addr = std::env::var("CXDB_TEST_ADDR").unwrap_or_else(|_| "127.0.0.1:9009".to_string());
    let http_base = std::env::var("CXDB_TEST_HTTP_ADDR")
        .unwrap_or_else(|_| "http://127.0.0.1:9010".to_string());

    let client = dial(&addr, Vec::new()).expect("dial failed");
    let ctx = RequestContext::background();
    let head = client
        .create_context(&ctx, 0)
        .expect("create context failed");

    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    std::fs::create_dir_all(temp_dir.path().join("src")).unwrap();
    std::fs::write(temp_dir.path().join("README.md"), "# Test Project").unwrap();
    std::fs::write(
        temp_dir.path().join("src").join("main.go"),
        "package main\n",
    )
    .unwrap();

    let snapshot = fstree::capture(temp_dir.path(), Vec::<fstree::SnapshotOption>::new())
        .expect("capture failed");
    let _upload = snapshot.upload(&ctx, &client).expect("upload failed");

    let payload = encode_msgpack(&new_user_input("Initial", Vec::new())).unwrap();
    let append = client
        .append_turn_with_fs(
            &ctx,
            &AppendRequest::new(
                head.context_id,
                TypeIDConversationItem,
                TypeVersionConversationItem,
                payload,
            ),
            Some(snapshot.root_hash),
        )
        .expect("append with fs failed");

    let listing = http_get_json(&format!("{http_base}/v1/turns/{}/fs", append.turn_id));
    let names = extract_names(&listing);
    assert!(names.contains(&"README.md".to_string()));
    assert!(names.contains(&"src".to_string()));

    let readme = ureq::get(&format!(
        "{http_base}/v1/turns/{}/fs/README.md",
        append.turn_id
    ))
    .call()
    .expect("readme request")
    .into_string()
    .expect("readme body");
    assert_eq!(readme, "# Test Project");

    let payload = encode_msgpack(&new_user_input("Followup", Vec::new())).unwrap();
    let append2 = client
        .append_turn(
            &ctx,
            &AppendRequest::new(
                head.context_id,
                TypeIDConversationItem,
                TypeVersionConversationItem,
                payload,
            ),
        )
        .expect("append without fs failed");

    let listing2 = http_get_json(&format!("{http_base}/v1/turns/{}/fs", append2.turn_id));
    let names2 = extract_names(&listing2);
    assert!(names2.contains(&"README.md".to_string()));
    assert!(names2.contains(&"src".to_string()));
}

fn http_get_json(url: &str) -> Value {
    let body = ureq::get(url)
        .call()
        .expect("http get")
        .into_string()
        .expect("json body");
    serde_json::from_str(&body).expect("parse json body")
}

fn extract_names(value: &Value) -> Vec<String> {
    let entries = value
        .get("entries")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut names: Vec<String> = entries
        .iter()
        .filter_map(|entry| {
            entry
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();
    names.sort();
    names
}
