// Copyright 2025 StrongDM Inc
// SPDX-License-Identifier: Apache-2.0

use std::io::{Read, Write};
use std::time::Duration;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::error::{Error, Result};

pub const MSG_HELLO: u16 = 1;
pub const MSG_CTX_CREATE: u16 = 2;
pub const MSG_CTX_FORK: u16 = 3;
pub const MSG_GET_HEAD: u16 = 4;
pub const MSG_APPEND_TURN: u16 = 5;
pub const MSG_GET_LAST: u16 = 6;
pub const MSG_GET_BLOB: u16 = 9;
pub const MSG_ATTACH_FS: u16 = 10;
pub const MSG_PUT_BLOB: u16 = 11;
pub const MSG_ERROR: u16 = 255;

pub const ENCODING_MSGPACK: u32 = 1;
pub const COMPRESSION_NONE: u32 = 0;
pub const COMPRESSION_ZSTD: u32 = 1;

pub const DEFAULT_DIAL_TIMEOUT: Duration = Duration::from_secs(5);
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub const MAX_FRAME_SIZE: u32 = 64 * 1024 * 1024; // 64 MiB

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub len: u32,
    pub msg_type: u16,
    pub flags: u16,
    pub req_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub header: FrameHeader,
    pub payload: Vec<u8>,
}

pub fn write_frame<W: Write>(
    writer: &mut W,
    msg_type: u16,
    flags: u16,
    req_id: u64,
    payload: &[u8],
) -> Result<()> {
    writer.write_u32::<LittleEndian>(payload.len() as u32)?;
    writer.write_u16::<LittleEndian>(msg_type)?;
    writer.write_u16::<LittleEndian>(flags)?;
    writer.write_u64::<LittleEndian>(req_id)?;
    writer.write_all(payload)?;
    Ok(())
}

pub fn read_frame<R: Read>(reader: &mut R) -> Result<Frame> {
    let len = match reader.read_u32::<LittleEndian>() {
        Ok(v) => v,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::UnexpectedEof {
                return Err(Error::invalid_response("frame header truncated"));
            }
            return Err(Error::Io(err));
        }
    };

    if len > MAX_FRAME_SIZE {
        return Err(Error::invalid_response(format!(
            "frame size {} exceeds maximum {}",
            len, MAX_FRAME_SIZE
        )));
    }

    let msg_type = reader
        .read_u16::<LittleEndian>()
        .map_err(map_header_error)?;
    let flags = reader
        .read_u16::<LittleEndian>()
        .map_err(map_header_error)?;
    let req_id = reader
        .read_u64::<LittleEndian>()
        .map_err(map_header_error)?;

    let mut payload = vec![0u8; len as usize];
    if let Err(err) = reader.read_exact(&mut payload) {
        if err.kind() == std::io::ErrorKind::UnexpectedEof {
            return Err(Error::invalid_response("frame payload truncated"));
        }
        return Err(Error::Io(err));
    }

    Ok(Frame {
        header: FrameHeader {
            len,
            msg_type,
            flags,
            req_id,
        },
        payload,
    })
}

fn map_header_error(err: std::io::Error) -> Error {
    if err.kind() == std::io::ErrorKind::UnexpectedEof {
        Error::invalid_response("frame header truncated")
    } else {
        Error::Io(err)
    }
}
