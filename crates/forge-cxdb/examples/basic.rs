// Copyright 2025 StrongDM Inc
// SPDX-License-Identifier: Apache-2.0

use cxdb::types::{new_user_input, TypeIDConversationItem, TypeVersionConversationItem};
use cxdb::{dial, encode_msgpack, AppendRequest, GetLastOptions, RequestContext};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = std::env::var("CXDB_ADDR").unwrap_or_else(|_| "127.0.0.1:9009".to_string());
    let client = dial(&addr, Vec::new())?;
    let ctx = RequestContext::background();

    let head = client.create_context(&ctx, 0)?;
    let payload = encode_msgpack(&new_user_input("Hello from Rust", Vec::new()))?;
    let append = client.append_turn(
        &ctx,
        &AppendRequest::new(
            head.context_id,
            TypeIDConversationItem,
            TypeVersionConversationItem,
            payload,
        ),
    )?;

    let turns = client.get_last(&ctx, head.context_id, GetLastOptions::default())?;
    println!(
        "context_id={}, appended turn_id={}, fetched {} turns",
        head.context_id,
        append.turn_id,
        turns.len()
    );
    Ok(())
}
