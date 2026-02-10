// Copyright 2025 StrongDM Inc
// SPDX-License-Identifier: Apache-2.0

#[cfg(test)]
use serde::Deserialize;

#[cfg(test)]
#[derive(Debug, Deserialize)]
pub struct Fixture {
    #[allow(dead_code)]
    pub name: String,
    pub msg_type: u16,
    pub flags: u16,
    pub payload_hex: String,
    #[allow(dead_code)]
    pub notes: Option<String>,
}

#[cfg(test)]
pub fn load_fixture(name: &str) -> Fixture {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let path = dir.join(format!("{name}.json"));
    let data = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read fixture {name}: {err}"));
    serde_json::from_str(&data)
        .unwrap_or_else(|err| panic!("failed to parse fixture {name}: {err}"))
}

#[cfg(test)]
pub fn decode_hex(hex_str: &str) -> Vec<u8> {
    hex::decode(hex_str).unwrap_or_else(|err| panic!("hex decode failed: {err}"))
}
