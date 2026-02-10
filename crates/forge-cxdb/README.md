# cxdb (Rust client)

Rust-native CXDB client with 1:1 wire parity to the Go client. This crate provides a synchronous TCP/TLS client, reconnection wrapper, filesystem snapshot helpers (`fstree`), and canonical conversation types.

## Endpoint topology and trust model

- Binary protocol endpoint (default `127.0.0.1:9009`) is the primary write plane:
  - context lifecycle (`CTX_CREATE`, `CTX_FORK`)
  - turn append/read (`APPEND_TURN`, `GET_HEAD`, `GET_LAST`)
  - blob/fs operations (`PUT_BLOB`, `GET_BLOB`, `ATTACH_FS`)
- HTTP endpoint (commonly `http://127.0.0.1:9010`) is the projection/registry plane:
  - context turn listing with cursor paging
  - registry bundle publish/read
- Production guidance:
  - keep binary endpoints private and restricted to trusted service networks
  - enforce TLS for transport, or equivalent encrypted/private overlay plus strict ACLs
  - place HTTP endpoints behind authenticated gateways and audit controls

## Quick Start

```rust
use cxdb::types::{new_user_input, TypeIDConversationItem, TypeVersionConversationItem};
use cxdb::{dial, encode_msgpack, AppendRequest, GetLastOptions, RequestContext};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = dial("127.0.0.1:9009", Vec::new())?;
    let ctx = RequestContext::background();

    let head = client.create_context(&ctx, 0)?;
    let payload = encode_msgpack(&new_user_input("Hello", Vec::new()))?;
    client.append_turn(
        &ctx,
        &AppendRequest::new(head.context_id, TypeIDConversationItem, TypeVersionConversationItem, payload),
    )?;

    let turns = client.get_last(&ctx, head.context_id, GetLastOptions::default())?;
    println!("fetched {} turns", turns.len());
    Ok(())
}
```

## Fstree snapshots

```rust
use cxdb::fstree;
use cxdb::types::{new_user_input, TypeIDConversationItem, TypeVersionConversationItem};
use cxdb::{dial, encode_msgpack, AppendRequest, RequestContext};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = dial("127.0.0.1:9009", Vec::new())?;
    let ctx = RequestContext::background();
    let head = client.create_context(&ctx, 0)?;

    let snapshot = fstree::capture(".", vec![fstree::with_exclude(vec![".git", "target"])])?;
    snapshot.upload(&ctx, &client)?;

    let payload = encode_msgpack(&new_user_input("Snapshot attached", Vec::new()))?;
    client.append_turn_with_fs(
        &ctx,
        &AppendRequest::new(head.context_id, TypeIDConversationItem, TypeVersionConversationItem, payload),
        Some(snapshot.root_hash),
    )?;
    Ok(())
}
```

## Reconnecting client

```rust
use cxdb::{dial_reconnecting, RequestContext, ReconnectOption};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = dial_reconnecting(
        "127.0.0.1:9009",
        Vec::<ReconnectOption>::new(),
        Vec::new(),
    )?;
    let ctx = RequestContext::background();
    let _ = client.create_context(&ctx, 0)?;
    Ok(())
}
```

## Msgpack helpers

- `encode_msgpack` emits deterministic map ordering (matching Go’s `SetSortMapKeys(true)`).
- Struct field tags use digit-strings (e.g., `"1"`, `"30"`) so encoded payloads match Go.
- Optional fields serialize as explicit `nil`, matching Go’s msgpack behavior.

## Examples

Run the bundled examples from this crate:

```bash
cargo run --example basic
cargo run --example fstree_snapshot
```

## Integration tests

Integration tests are gated by environment variables:

```bash
export CXDB_INTEGRATION=1
export CXDB_TEST_ADDR=127.0.0.1:9009
export CXDB_TEST_HTTP_ADDR=http://127.0.0.1:9010
cargo test -p cxdb
```

## Incident debugging checklist

- Append path failures:
  1. Confirm binary endpoint reachability and TLS trust chain.
  2. Validate idempotency key reuse on retries.
  3. Check context/parent ids for ordering or conversion errors.
- Projection path failures:
  1. Confirm HTTP endpoint auth/routing.
  2. Validate cursor values and limit bounds.
  3. Distinguish ingest lag from read path bugs by checking binary write success.
- Registry mismatch:
  1. Verify bundle exists at expected `bundle_id`.
  2. Confirm writer `type_id`/`type_version` align with bundle schema.
  3. Re-publish bundle before introducing new schema-version writes.
- FS snapshot/attachment failures:
  1. Check `fstree` capture policy limits/excludes.
  2. Verify blob upload completed and hashes are stable.
  3. Attach only known `fs_root_hash` values to existing turns.

## Parity notes

- Wire format and message types follow `docs/protocol.md` and the Go client implementation.
- Fstree tree serialization is validated against Go-generated fixtures.
- Canonical types use the same msgpack tags and optional-field semantics as Go.
