# forge-turnstore-cxdb (compat)

Compatibility shim for legacy imports.

- Re-exports `forge-cxdb-runtime`.
- Kept during CXDB-first migration to avoid breaking downstream code immediately.
- New code should depend on `forge-cxdb-runtime` directly.
