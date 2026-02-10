// Copyright 2025 StrongDM Inc
// SPDX-License-Identifier: Apache-2.0

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::Read;

use crate::client::{Client, RequestContext};
use crate::error::{Error, Result};
use crate::protocol::{ENCODING_MSGPACK, MSG_APPEND_TURN, MSG_GET_LAST};

#[derive(Debug, Clone)]
pub struct AppendRequest {
    pub context_id: u64,
    pub parent_turn_id: u64,
    pub type_id: String,
    pub type_version: u32,
    pub payload: Vec<u8>,
    pub idempotency_key: Vec<u8>,
    pub encoding: u32,
    pub compression: u32,
}

impl AppendRequest {
    pub fn new(
        context_id: u64,
        type_id: impl Into<String>,
        type_version: u32,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            context_id,
            parent_turn_id: 0,
            type_id: type_id.into(),
            type_version,
            payload,
            idempotency_key: Vec::new(),
            encoding: ENCODING_MSGPACK,
            compression: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRecord {
    pub turn_id: u64,
    pub parent_id: u64,
    pub depth: u32,
    pub type_id: String,
    pub type_version: u32,
    pub encoding: u32,
    pub compression: u32,
    pub payload_hash: [u8; 32],
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendResult {
    pub context_id: u64,
    pub turn_id: u64,
    pub depth: u32,
    pub payload_hash: [u8; 32],
}

#[derive(Debug, Clone, Copy)]
pub struct GetLastOptions {
    pub limit: u32,
    pub include_payload: bool,
}

impl Default for GetLastOptions {
    fn default() -> Self {
        Self {
            limit: 10,
            include_payload: false,
        }
    }
}

impl Client {
    pub fn append_turn(&self, ctx: &RequestContext, req: &AppendRequest) -> Result<AppendResult> {
        let encoding = if req.encoding == 0 {
            ENCODING_MSGPACK
        } else {
            req.encoding
        };

        let hash = blake3::hash(&req.payload);

        let mut payload = Vec::with_capacity(128 + req.payload.len());
        payload.write_u64::<LittleEndian>(req.context_id)?;
        payload.write_u64::<LittleEndian>(req.parent_turn_id)?;

        payload.write_u32::<LittleEndian>(req.type_id.len() as u32)?;
        payload.extend_from_slice(req.type_id.as_bytes());
        payload.write_u32::<LittleEndian>(req.type_version)?;

        payload.write_u32::<LittleEndian>(encoding)?;
        payload.write_u32::<LittleEndian>(req.compression)?;
        payload.write_u32::<LittleEndian>(req.payload.len() as u32)?; // uncompressed len
        payload.extend_from_slice(hash.as_bytes());

        payload.write_u32::<LittleEndian>(req.payload.len() as u32)?;
        payload.extend_from_slice(&req.payload);

        payload.write_u32::<LittleEndian>(req.idempotency_key.len() as u32)?;
        if !req.idempotency_key.is_empty() {
            payload.extend_from_slice(&req.idempotency_key);
        }

        let frame = self.send_request(ctx, MSG_APPEND_TURN, &payload)?;
        parse_append_result(&frame.payload)
    }

    pub fn get_last(
        &self,
        ctx: &RequestContext,
        context_id: u64,
        opts: GetLastOptions,
    ) -> Result<Vec<TurnRecord>> {
        let limit = if opts.limit == 0 { 10 } else { opts.limit };
        let mut payload = Vec::with_capacity(16);
        payload.write_u64::<LittleEndian>(context_id)?;
        payload.write_u32::<LittleEndian>(limit)?;
        payload.write_u32::<LittleEndian>(if opts.include_payload { 1 } else { 0 })?;

        let frame = self.send_request(ctx, MSG_GET_LAST, &payload)?;
        parse_turn_records(&frame.payload)
    }
}

fn parse_append_result(payload: &[u8]) -> Result<AppendResult> {
    if payload.len() < 52 {
        return Err(Error::invalid_response(format!(
            "append response too short ({} bytes)",
            payload.len()
        )));
    }
    let mut cursor = std::io::Cursor::new(payload);
    let context_id = cursor.read_u64::<LittleEndian>()?;
    let turn_id = cursor.read_u64::<LittleEndian>()?;
    let depth = cursor.read_u32::<LittleEndian>()?;
    let mut hash = [0u8; 32];
    cursor.read_exact(&mut hash)?;
    Ok(AppendResult {
        context_id,
        turn_id,
        depth,
        payload_hash: hash,
    })
}

fn parse_turn_records(payload: &[u8]) -> Result<Vec<TurnRecord>> {
    if payload.len() < 4 {
        return Err(Error::invalid_response("turn records too short"));
    }

    let mut cursor = std::io::Cursor::new(payload);
    let count = cursor.read_u32::<LittleEndian>()?;
    let mut records = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let turn_id = cursor.read_u64::<LittleEndian>()?;
        let parent_id = cursor.read_u64::<LittleEndian>()?;
        let depth = cursor.read_u32::<LittleEndian>()?;

        let type_len = cursor.read_u32::<LittleEndian>()? as usize;
        let mut type_bytes = vec![0u8; type_len];
        cursor.read_exact(&mut type_bytes)?;
        let type_id = String::from_utf8(type_bytes)
            .map_err(|_| Error::invalid_response("type_id not utf8"))?;

        let type_version = cursor.read_u32::<LittleEndian>()?;
        let encoding = cursor.read_u32::<LittleEndian>()?;
        let compression = cursor.read_u32::<LittleEndian>()?;

        let _uncompressed_len = cursor.read_u32::<LittleEndian>()?;
        let mut payload_hash = [0u8; 32];
        cursor.read_exact(&mut payload_hash)?;

        let payload_len = cursor.read_u32::<LittleEndian>()? as usize;
        let mut payload_bytes = vec![0u8; payload_len];
        cursor.read_exact(&mut payload_bytes)?;

        records.push(TurnRecord {
            turn_id,
            parent_id,
            depth,
            type_id,
            type_version,
            encoding,
            compression,
            payload_hash,
            payload: payload_bytes,
        });
    }

    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{decode_hex, load_fixture};

    fn build_append_payload(req: &AppendRequest) -> Vec<u8> {
        let encoding = if req.encoding == 0 {
            ENCODING_MSGPACK
        } else {
            req.encoding
        };
        let hash = blake3::hash(&req.payload);
        let mut payload = Vec::new();
        payload.write_u64::<LittleEndian>(req.context_id).unwrap();
        payload
            .write_u64::<LittleEndian>(req.parent_turn_id)
            .unwrap();
        payload
            .write_u32::<LittleEndian>(req.type_id.len() as u32)
            .unwrap();
        payload.extend_from_slice(req.type_id.as_bytes());
        payload.write_u32::<LittleEndian>(req.type_version).unwrap();
        payload.write_u32::<LittleEndian>(encoding).unwrap();
        payload.write_u32::<LittleEndian>(req.compression).unwrap();
        payload
            .write_u32::<LittleEndian>(req.payload.len() as u32)
            .unwrap();
        payload.extend_from_slice(hash.as_bytes());
        payload
            .write_u32::<LittleEndian>(req.payload.len() as u32)
            .unwrap();
        payload.extend_from_slice(&req.payload);
        payload
            .write_u32::<LittleEndian>(req.idempotency_key.len() as u32)
            .unwrap();
        if !req.idempotency_key.is_empty() {
            payload.extend_from_slice(&req.idempotency_key);
        }
        payload
    }

    #[test]
    fn append_payloads_match_fixtures() {
        let fixture = load_fixture("append_parent0");
        assert_eq!(fixture.msg_type, MSG_APPEND_TURN);
        assert_eq!(fixture.flags, 0);
        let req = AppendRequest {
            context_id: 1,
            parent_turn_id: 0,
            type_id: "cxdb.ConversationItem".into(),
            type_version: 3,
            payload: vec![0x91, 0x01],
            idempotency_key: Vec::new(),
            encoding: ENCODING_MSGPACK,
            compression: 0,
        };
        assert_eq!(decode_hex(&fixture.payload_hex), build_append_payload(&req));

        let fixture = load_fixture("append_parent7");
        assert_eq!(fixture.msg_type, MSG_APPEND_TURN);
        assert_eq!(fixture.flags, 0);
        let req = AppendRequest {
            context_id: 1,
            parent_turn_id: 7,
            type_id: "cxdb.ConversationItem".into(),
            type_version: 3,
            payload: vec![0x91, 0x02],
            idempotency_key: Vec::new(),
            encoding: ENCODING_MSGPACK,
            compression: 0,
        };
        assert_eq!(decode_hex(&fixture.payload_hex), build_append_payload(&req));

        let fixture = load_fixture("append_idempotent");
        assert_eq!(fixture.msg_type, MSG_APPEND_TURN);
        assert_eq!(fixture.flags, 0);
        let req = AppendRequest {
            context_id: 1,
            parent_turn_id: 0,
            type_id: "cxdb.ConversationItem".into(),
            type_version: 3,
            payload: vec![0x91, 0x03],
            idempotency_key: b"idem-1".to_vec(),
            encoding: ENCODING_MSGPACK,
            compression: 0,
        };
        assert_eq!(decode_hex(&fixture.payload_hex), build_append_payload(&req));
    }

    #[test]
    fn get_last_payloads_match_fixtures() {
        let fixture = load_fixture("get_last_default");
        assert_eq!(fixture.msg_type, MSG_GET_LAST);
        let mut payload = Vec::new();
        payload.write_u64::<LittleEndian>(1).unwrap();
        payload.write_u32::<LittleEndian>(10).unwrap();
        payload.write_u32::<LittleEndian>(0).unwrap();
        assert_eq!(decode_hex(&fixture.payload_hex), payload);

        let fixture = load_fixture("get_last_payload");
        assert_eq!(fixture.msg_type, MSG_GET_LAST);
        let mut payload = Vec::new();
        payload.write_u64::<LittleEndian>(1).unwrap();
        payload.write_u32::<LittleEndian>(5).unwrap();
        payload.write_u32::<LittleEndian>(1).unwrap();
        assert_eq!(decode_hex(&fixture.payload_hex), payload);
    }
}
