# P06: Model Catalog
_Complete_

**Status**
- Done (2026-02-09)

**Goal**
Ship a provider model catalog and helpers for capability-based lookup.

**Scope**
- `ModelInfo` struct and catalog data file (JSON or TOML) stored in the crate.
- `get_model_info`, `list_models`, `get_latest_model` functions.
- Data loading with graceful fallback for unknown models.
- Update workflow for refreshing the catalog.

**Out of Scope**
- Auto-fetching from provider APIs (can be future work).

**Deliverables**
- Catalog file with the latest model entries specified in `spec/01-unified-llm-spec.md` section 2.9.
- Helper functions with tests for filtering and selection.

**Acceptance**
- Catalog loads deterministically in offline environments.
- `get_latest_model` returns expected models for each provider.
- Unknown model strings are still accepted by request types.

**Completed**
1. Added `ModelInfo` and an embedded JSON catalog data file at `crates/forge-llm/src/catalog_models.json`.
2. Implemented `get_model_info`, `list_models`, and `get_latest_model` in `crates/forge-llm/src/catalog.rs`.
3. Added capability filtering support (`tools`, `vision`, `reasoning`) for `get_latest_model`.
4. Added deterministic unit tests for filtering, latest selection, and unknown model/capability behavior.
