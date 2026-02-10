// Copyright 2025 StrongDM Inc
// SPDX-License-Identifier: Apache-2.0

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::Read;

use crate::client::{Client, RequestContext};
use crate::error::{Error, Result};
use crate::protocol::{ENCODING_MSGPACK, MSG_APPEND_TURN, MSG_ATTACH_FS, MSG_PUT_BLOB};
use crate::turn::{AppendRequest, AppendResult};

#[derive(Debug, Clone)]
pub struct AttachFsRequest {
    pub turn_id: u64,
    pub fs_root_hash: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachFsResult {
    pub turn_id: u64,
    pub fs_root_hash: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct PutBlobRequest {
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutBlobResult {
    pub hash: [u8; 32],
    pub was_new: bool,
}

impl Client {
    pub fn attach_fs(&self, ctx: &RequestContext, req: &AttachFsRequest) -> Result<AttachFsResult> {
        let mut payload = Vec::with_capacity(40);
        payload.write_u64::<LittleEndian>(req.turn_id)?;
        payload.extend_from_slice(&req.fs_root_hash);

        let frame = self.send_request(ctx, MSG_ATTACH_FS, &payload)?;
        if frame.payload.len() < 40 {
            return Err(Error::invalid_response(format!(
                "attach fs response too short ({} bytes)",
                frame.payload.len()
            )));
        }

        let mut cursor = std::io::Cursor::new(frame.payload);
        let turn_id = cursor.read_u64::<LittleEndian>()?;
        let mut hash = [0u8; 32];
        cursor.read_exact(&mut hash)?;

        Ok(AttachFsResult {
            turn_id,
            fs_root_hash: hash,
        })
    }

    pub fn put_blob(&self, ctx: &RequestContext, req: &PutBlobRequest) -> Result<PutBlobResult> {
        let hash = blake3::hash(&req.data);
        let mut payload = Vec::with_capacity(36 + req.data.len());
        payload.extend_from_slice(hash.as_bytes());
        payload.write_u32::<LittleEndian>(req.data.len() as u32)?;
        payload.extend_from_slice(&req.data);

        let frame = self.send_request(ctx, MSG_PUT_BLOB, &payload)?;
        if frame.payload.len() < 33 {
            return Err(Error::invalid_response(format!(
                "put blob response too short ({} bytes)",
                frame.payload.len()
            )));
        }
        let mut hash_bytes = [0u8; 32];
        hash_bytes.copy_from_slice(&frame.payload[0..32]);
        let was_new = frame.payload[32] == 1;
        Ok(PutBlobResult {
            hash: hash_bytes,
            was_new,
        })
    }

    pub fn put_blob_if_absent(
        &self,
        ctx: &RequestContext,
        data: Vec<u8>,
    ) -> Result<([u8; 32], bool)> {
        let result = self.put_blob(ctx, &PutBlobRequest { data })?;
        Ok((result.hash, result.was_new))
    }

    pub fn append_turn_with_fs(
        &self,
        ctx: &RequestContext,
        req: &AppendRequest,
        fs_root_hash: Option<[u8; 32]>,
    ) -> Result<AppendResult> {
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
        payload.write_u32::<LittleEndian>(req.payload.len() as u32)?;
        payload.extend_from_slice(hash.as_bytes());
        payload.write_u32::<LittleEndian>(req.payload.len() as u32)?;
        payload.extend_from_slice(&req.payload);
        payload.write_u32::<LittleEndian>(req.idempotency_key.len() as u32)?;
        if !req.idempotency_key.is_empty() {
            payload.extend_from_slice(&req.idempotency_key);
        }

        let mut flags = 0u16;
        if let Some(hash) = fs_root_hash {
            flags |= 1;
            payload.extend_from_slice(&hash);
        }

        let frame = self.send_request_with_flags(ctx, MSG_APPEND_TURN, flags, &payload)?;
        if frame.payload.len() < 52 {
            return Err(Error::invalid_response(format!(
                "append response too short ({} bytes)",
                frame.payload.len()
            )));
        }
        let mut cursor = std::io::Cursor::new(frame.payload);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{decode_hex, load_fixture};

    fn build_append_payload(req: &AppendRequest, fs_root_hash: Option<[u8; 32]>) -> Vec<u8> {
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
        if let Some(hash) = fs_root_hash {
            payload.extend_from_slice(&hash);
        }
        payload
    }

    #[test]
    fn fs_payloads_match_fixtures() {
        let fixture = load_fixture("attach_fs");
        assert_eq!(fixture.msg_type, MSG_ATTACH_FS);
        assert_eq!(fixture.flags, 0);
        let mut payload = Vec::new();
        payload.write_u64::<LittleEndian>(99).unwrap();
        payload.extend_from_slice(&[0xAA; 32]);
        assert_eq!(decode_hex(&fixture.payload_hex), payload);

        let fixture = load_fixture("put_blob");
        assert_eq!(fixture.msg_type, MSG_PUT_BLOB);
        assert_eq!(fixture.flags, 0);
        let data = b"hello blob";
        let hash = blake3::hash(data);
        let mut payload = Vec::new();
        payload.extend_from_slice(hash.as_bytes());
        payload
            .write_u32::<LittleEndian>(data.len() as u32)
            .unwrap();
        payload.extend_from_slice(data);
        assert_eq!(decode_hex(&fixture.payload_hex), payload);

        let fixture = load_fixture("append_with_fs");
        assert_eq!(fixture.msg_type, MSG_APPEND_TURN);
        assert_eq!(fixture.flags, 1);
        let req = AppendRequest {
            context_id: 1,
            parent_turn_id: 0,
            type_id: "cxdb.ConversationItem".into(),
            type_version: 3,
            payload: vec![0x91, 0x04],
            idempotency_key: Vec::new(),
            encoding: ENCODING_MSGPACK,
            compression: 0,
        };
        let payload = build_append_payload(&req, Some([0xBB; 32]));
        assert_eq!(decode_hex(&fixture.payload_hex), payload);
    }
}
