# CXDB Rust Client Parity Fixtures

This directory contains golden vectors generated from the Go client to ensure
Rust request payloads match byte-for-byte.

## Format
- One JSON file per operation.
- Fields:
  - `name`: short identifier
  - `msg_type`: numeric message type
  - `flags`: frame flags used for the request
  - `payload_hex`: lowercase hex of the request payload bytes
  - `notes`: optional freeform string

## Generation
Run the Go fixture generator (added under `clients/go/cmd/cxdb-fixtures`) to
regenerate fixtures deterministically. The generator writes JSON files into
this directory.

## Consumption
Rust tests load fixtures from this directory and compare generated payloads
against `payload_hex`.
