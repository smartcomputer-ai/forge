//! Shared rendering for catalog context entries.
//!
//! A catalog that changes is not rewritten in place: the engine keeps the
//! earlier version active and appends the new one with `supersedes` set, so
//! the rendered prefix never moves and the provider prompt cache holds. The
//! earlier version must therefore render byte-for-byte as it always did —
//! only the successor announces the update.

use engine::{BlobRef, ContextEntry, ContextEntryKind, storage::BlobStore};

use crate::error::LlmAdapterResult;

/// Line prepended to a catalog that supersedes an earlier version.
pub(crate) const CATALOG_UPDATE_HEADER: &str =
    "Updated catalog — this version supersedes the earlier one above; use this one.";

/// Wrap a rendered catalog body with the update header when the entry
/// supersedes an earlier version. The body of a non-superseding entry is
/// returned unchanged.
pub(crate) fn catalog_text(entry: &ContextEntry, body: String) -> String {
    if entry.supersedes.is_some() {
        format!("{CATALOG_UPDATE_HEADER}\n\n{body}")
    } else {
        body
    }
}

/// Render a client-owned catalog: its title as a heading, then the stored
/// text verbatim.
pub(crate) async fn external_catalog_text(
    blobs: &dyn BlobStore,
    entry: &ContextEntry,
    content_ref: &BlobRef,
) -> LlmAdapterResult<String> {
    let title = match &entry.kind {
        ContextEntryKind::Catalog { title } => title.trim(),
        _ => "Catalog",
    };
    let text = crate::blob_io::read_text(blobs, content_ref).await?;
    Ok(catalog_text(entry, format!("{title}:\n\n{}", text.trim())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::{ContextEntryId, ContextEntrySource};

    fn entry(supersedes: Option<u64>) -> ContextEntry {
        ContextEntry {
            entry_id: ContextEntryId::new(7),
            key: None,
            kind: ContextEntryKind::Catalog {
                title: "Bot directory".to_owned(),
            },
            source: ContextEntrySource::ContextEdit,
            content_ref: BlobRef::from_bytes(b"body"),
            media_type: None,
            preview: None,
            provider_kind: None,
            provider_item_id: None,
            token_estimate: None,
            supersedes: supersedes.map(ContextEntryId::new),
        }
    }

    #[test]
    fn body_is_unchanged_without_supersedes() {
        assert_eq!(catalog_text(&entry(None), "body".to_owned()), "body");
    }

    #[test]
    fn superseding_entry_gets_the_update_header() {
        let text = catalog_text(&entry(Some(3)), "body".to_owned());
        assert!(text.starts_with(CATALOG_UPDATE_HEADER));
        assert!(text.ends_with("\n\nbody"));
    }
}
