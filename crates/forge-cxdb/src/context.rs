// Copyright 2025 StrongDM Inc
// SPDX-License-Identifier: Apache-2.0

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::client::{Client, RequestContext};
use crate::error::{Error, Result};
use crate::protocol::{MSG_CTX_CREATE, MSG_CTX_FORK, MSG_GET_HEAD};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextHead {
    pub context_id: u64,
    pub head_turn_id: u64,
    pub head_depth: u32,
}

impl Client {
    pub fn create_context(&self, ctx: &RequestContext, base_turn_id: u64) -> Result<ContextHead> {
        let mut payload = Vec::with_capacity(8);
        payload.write_u64::<LittleEndian>(base_turn_id)?;
        let frame = self.send_request(ctx, MSG_CTX_CREATE, &payload)?;
        parse_context_head(&frame.payload)
    }

    pub fn fork_context(&self, ctx: &RequestContext, base_turn_id: u64) -> Result<ContextHead> {
        let mut payload = Vec::with_capacity(8);
        payload.write_u64::<LittleEndian>(base_turn_id)?;
        let frame = self.send_request(ctx, MSG_CTX_FORK, &payload)?;
        parse_context_head(&frame.payload)
    }

    pub fn get_head(&self, ctx: &RequestContext, context_id: u64) -> Result<ContextHead> {
        let mut payload = Vec::with_capacity(8);
        payload.write_u64::<LittleEndian>(context_id)?;
        let frame = self.send_request(ctx, MSG_GET_HEAD, &payload)?;
        parse_context_head(&frame.payload)
    }
}

fn parse_context_head(payload: &[u8]) -> Result<ContextHead> {
    if payload.len() < 20 {
        return Err(Error::invalid_response(format!(
            "context head too short ({} bytes)",
            payload.len()
        )));
    }
    let mut cursor = std::io::Cursor::new(payload);
    Ok(ContextHead {
        context_id: cursor.read_u64::<LittleEndian>()?,
        head_turn_id: cursor.read_u64::<LittleEndian>()?,
        head_depth: cursor.read_u32::<LittleEndian>()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{decode_hex, load_fixture};

    fn payload_u64(value: u64) -> Vec<u8> {
        let mut payload = Vec::with_capacity(8);
        payload.write_u64::<LittleEndian>(value).unwrap();
        payload
    }

    #[test]
    fn context_payloads_match_fixtures() {
        let fixture = load_fixture("ctx_create_base0");
        assert_eq!(fixture.msg_type, MSG_CTX_CREATE);
        assert_eq!(decode_hex(&fixture.payload_hex), payload_u64(0));

        let fixture = load_fixture("ctx_fork_base123");
        assert_eq!(fixture.msg_type, MSG_CTX_FORK);
        assert_eq!(decode_hex(&fixture.payload_hex), payload_u64(123));

        let fixture = load_fixture("get_head_ctx42");
        assert_eq!(fixture.msg_type, MSG_GET_HEAD);
        assert_eq!(decode_hex(&fixture.payload_hex), payload_u64(42));
    }
}
