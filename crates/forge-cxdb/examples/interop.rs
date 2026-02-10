// Copyright 2025 StrongDM Inc
// SPDX-License-Identifier: Apache-2.0

use cxdb::{decode_msgpack, dial, encode_msgpack, AppendRequest, GetLastOptions, RequestContext};
use std::collections::BTreeMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| "help".to_string());
    match cmd.as_str() {
        "write" => write_flow(&mut args)?,
        "read" => read_flow(&mut args)?,
        _ => {
            eprintln!(
                "usage: interop write <addr> <role> <text> | interop read <addr> <context_id>"
            );
        }
    }
    Ok(())
}

fn write_flow(args: &mut impl Iterator<Item = String>) -> Result<(), Box<dyn std::error::Error>> {
    let addr = args.next().unwrap_or_else(|| "127.0.0.1:9009".to_string());
    let role = args.next().unwrap_or_else(|| "user".to_string());
    let text = args.next().unwrap_or_else(|| "hello".to_string());

    let client = dial(&addr, Vec::new())?;
    let ctx = RequestContext::background();
    let head = client.create_context(&ctx, 0)?;

    let payload = encode_msgpack(&BTreeMap::from([(1u64, role), (2u64, text)]))?;
    let req = AppendRequest::new(head.context_id, "com.yourorg.ai.MessageTurn", 1, payload);
    let append = client.append_turn(&ctx, &req)?;

    println!("context_id={} turn_id={}", head.context_id, append.turn_id);
    Ok(())
}

fn read_flow(args: &mut impl Iterator<Item = String>) -> Result<(), Box<dyn std::error::Error>> {
    let addr = args.next().unwrap_or_else(|| "127.0.0.1:9009".to_string());
    let context_id: u64 = args
        .next()
        .ok_or("missing context_id")?
        .parse()
        .map_err(|_| "invalid context_id")?;

    let client = dial(&addr, Vec::new())?;
    let ctx = RequestContext::background();
    let turns = client.get_last(
        &ctx,
        context_id,
        GetLastOptions {
            limit: 1,
            include_payload: true,
        },
    )?;

    if let Some(turn) = turns.first() {
        let decoded = decode_msgpack(&turn.payload)?;
        let role = decoded.get(&1).and_then(|v| v.as_str()).unwrap_or("");
        let text = decoded.get(&2).and_then(|v| v.as_str()).unwrap_or("");
        println!("role={} text={}", role, text);
    } else {
        println!("no turns");
    }
    Ok(())
}
