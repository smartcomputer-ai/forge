// Copyright 2025 StrongDM Inc
// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;

use rmpv::Value;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_value::Value as SerdeValue;

use crate::error::{Error, Result};

pub fn encode_msgpack<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let value = serde_value::to_value(value)
        .map_err(|err| Error::invalid_response(format!("msgpack encode error: {err}")))?;
    let mut buf = Vec::new();
    write_serde_value(&mut buf, &value)
        .map_err(|err| Error::invalid_response(format!("msgpack encode error: {err}")))?;
    Ok(buf)
}

pub fn decode_msgpack(data: &[u8]) -> Result<BTreeMap<u64, Value>> {
    let mut cursor = std::io::Cursor::new(data);
    let value = rmpv::decode::read_value(&mut cursor)
        .map_err(|err| Error::invalid_response(format!("msgpack decode error: {err}")))?;
    let map = match value {
        Value::Map(entries) => entries,
        _ => return Err(Error::invalid_response("msgpack payload is not a map")),
    };

    let mut out = BTreeMap::new();
    for (k, v) in map {
        let key = match k {
            Value::Integer(i) => i
                .as_u64()
                .ok_or_else(|| Error::invalid_response("invalid map key"))?,
            Value::String(s) => s
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or_else(|| Error::invalid_response("invalid map key"))?,
            _ => return Err(Error::invalid_response("invalid map key")),
        };
        out.insert(key, v);
    }
    Ok(out)
}

pub fn decode_msgpack_into<T: DeserializeOwned>(data: &[u8]) -> Result<T> {
    match rmp_serde::from_slice::<T>(data) {
        Ok(value) => Ok(value),
        Err(_) => {
            let mut cursor = std::io::Cursor::new(data);
            let mut value = rmpv::decode::read_value(&mut cursor)
                .map_err(|err| Error::invalid_response(format!("msgpack decode error: {err}")))?;
            normalize_map_keys_to_string(&mut value);
            rmpv::ext::from_value::<T>(value)
                .map_err(|err| Error::invalid_response(format!("msgpack decode error: {err}")))
        }
    }
}

#[allow(non_snake_case)]
pub fn EncodeMsgpack<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    encode_msgpack(value)
}

#[allow(non_snake_case)]
pub fn DecodeMsgpack(data: &[u8]) -> Result<BTreeMap<u64, Value>> {
    decode_msgpack(data)
}

#[allow(non_snake_case)]
pub fn DecodeMsgpackInto<T: DeserializeOwned>(data: &[u8]) -> Result<T> {
    decode_msgpack_into(data)
}

fn normalize_map_keys_to_string(value: &mut Value) {
    match value {
        Value::Map(entries) => {
            for (k, v) in entries.iter_mut() {
                if let Value::Integer(i) = k {
                    if let Some(num) = i.as_u64() {
                        *k = Value::String(num.to_string().into());
                    }
                }
                normalize_map_keys_to_string(v);
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                normalize_map_keys_to_string(item);
            }
        }
        _ => {}
    }
}

fn write_serde_value<W: std::io::Write>(writer: &mut W, value: &SerdeValue) -> std::io::Result<()> {
    use rmp::encode;

    match value {
        SerdeValue::Bool(v) => encode::write_bool(writer, *v),
        SerdeValue::U8(v) => encode::write_u8(writer, *v).map_err(std::io::Error::from),
        SerdeValue::U16(v) => encode::write_u16(writer, *v).map_err(std::io::Error::from),
        SerdeValue::U32(v) => encode::write_u32(writer, *v).map_err(std::io::Error::from),
        SerdeValue::U64(v) => encode::write_u64(writer, *v).map_err(std::io::Error::from),
        SerdeValue::I8(v) => encode::write_i8(writer, *v).map_err(std::io::Error::from),
        SerdeValue::I16(v) => encode::write_i16(writer, *v).map_err(std::io::Error::from),
        SerdeValue::I32(v) => encode::write_i32(writer, *v).map_err(std::io::Error::from),
        SerdeValue::I64(v) => encode::write_i64(writer, *v).map_err(std::io::Error::from),
        SerdeValue::F32(v) => encode::write_f32(writer, *v).map_err(std::io::Error::from),
        SerdeValue::F64(v) => encode::write_f64(writer, *v).map_err(std::io::Error::from),
        SerdeValue::Char(c) => {
            let mut buf = [0u8; 4];
            let s = c.encode_utf8(&mut buf);
            encode::write_str(writer, s).map_err(std::io::Error::from)
        }
        SerdeValue::String(s) => encode::write_str(writer, s).map_err(std::io::Error::from),
        SerdeValue::Unit => encode::write_nil(writer),
        SerdeValue::Option(opt) => match opt {
            Some(v) => write_serde_value(writer, v),
            None => encode::write_nil(writer),
        },
        SerdeValue::Newtype(inner) => write_serde_value(writer, inner),
        SerdeValue::Seq(items) => {
            encode::write_array_len(writer, items.len() as u32).map_err(std::io::Error::from)?;
            for item in items {
                write_serde_value(writer, item)?;
            }
            Ok(())
        }
        SerdeValue::Map(map) => {
            encode::write_map_len(writer, map.len() as u32).map_err(std::io::Error::from)?;
            let mut entries: Vec<(&SerdeValue, &SerdeValue)> = map.iter().collect();
            entries.sort_by(|(ka, _), (kb, _)| encoded_key_cmp(ka, kb));
            for (key, value) in entries {
                write_serde_value(writer, key)?;
                write_serde_value(writer, value)?;
            }
            Ok(())
        }
        SerdeValue::Bytes(bytes) => encode::write_bin(writer, bytes).map_err(std::io::Error::from),
    }
}

fn encoded_key_cmp(a: &SerdeValue, b: &SerdeValue) -> std::cmp::Ordering {
    let mut buf_a = Vec::new();
    let mut buf_b = Vec::new();
    let _ = write_serde_value(&mut buf_a, a);
    let _ = write_serde_value(&mut buf_b, b);
    buf_a.cmp(&buf_b)
}
