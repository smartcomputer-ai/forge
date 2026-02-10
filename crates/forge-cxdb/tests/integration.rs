// Copyright 2025 StrongDM Inc
// SPDX-License-Identifier: Apache-2.0

use cxdb::{dial, RequestContext};

#[test]
fn integration_create_context_smoke() {
    if std::env::var("CXDB_INTEGRATION").is_err() {
        eprintln!("CXDB_INTEGRATION not set; skipping integration test");
        return;
    }

    let addr = std::env::var("CXDB_TEST_ADDR").unwrap_or_else(|_| "127.0.0.1:9009".to_string());
    let client = dial(&addr, Vec::new()).expect("dial failed");
    let ctx = RequestContext::background();
    let head = client
        .create_context(&ctx, 0)
        .expect("create context failed");
    assert!(head.context_id > 0);
}
