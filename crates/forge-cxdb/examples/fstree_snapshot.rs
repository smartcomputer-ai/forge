// Copyright 2025 StrongDM Inc
// SPDX-License-Identifier: Apache-2.0

use cxdb::fstree;
use cxdb::types::{new_user_input, TypeIDConversationItem, TypeVersionConversationItem};
use cxdb::{dial, encode_msgpack, AppendRequest, RequestContext};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = std::env::var("CXDB_ADDR").unwrap_or_else(|_| "127.0.0.1:9009".to_string());
    let root = std::env::var("CXDB_FS_ROOT").unwrap_or_else(|_| ".".to_string());

    let client = dial(&addr, Vec::new())?;
    let ctx = RequestContext::background();
    let head = client.create_context(&ctx, 0)?;

    let snapshot = fstree::capture(&root, vec![fstree::with_exclude(vec![".git", "target"])])?;
    let upload = snapshot.upload(&ctx, &client)?;

    let payload = encode_msgpack(&new_user_input("Captured snapshot", Vec::new()))?;
    let append = client.append_turn_with_fs(
        &ctx,
        &AppendRequest::new(
            head.context_id,
            TypeIDConversationItem,
            TypeVersionConversationItem,
            payload,
        ),
        Some(snapshot.root_hash),
    )?;

    println!(
        "turn_id={} fs_root={} trees_uploaded={} files_uploaded={}",
        append.turn_id,
        to_hex(&snapshot.root_hash),
        upload.trees_uploaded,
        upload.files_uploaded
    );
    Ok(())
}

fn to_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
