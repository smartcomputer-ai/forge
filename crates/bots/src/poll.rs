//! Poll triggers: the item list of a payload, each item's identity under
//! the trigger's cursor discipline, and the diff against the stored cursor.
//! First contact baselines and delivers nothing — enabling a poll against
//! a feed with a deep history must not flood the bot — and a fire delivers
//! at most [`MAX_POLL_ITEMS_PER_FIRE`] items, advancing the cursor only
//! over what it delivered so the rest wait for the next fire.

use std::collections::HashSet;

use api::{PollCursorSpec, PollCursorState};
use serde_json::{Number, Value};

/// Ids kept in an id-set cursor; older ids age out oldest-first.
pub const MAX_POLL_CURSOR_IDS: usize = 500;
/// New items admitted per fire; the rest wait for the next fire.
pub const MAX_POLL_ITEMS_PER_FIRE: usize = 100;
/// Consecutive failed fires before the trigger disables itself.
pub const MAX_POLL_CONSECUTIVE_FAILURES: u32 = 10;

const SUMMARY_FIELDS: [&str; 5] = ["summary", "title", "name", "subject", "message"];
const MAX_SUMMARY_CHARS: usize = 300;
const MAX_SUMMARY_KEY_CHARS: usize = 80;
const PAYLOAD_PREVIEW_CHARS: usize = 200;

/// Resolve a dot path: object keys and array indices, one segment each.
/// An empty segment, a missing key, or a scalar in the middle yields
/// `None`; an explicit `null` at the end is `Some(Null)`.
pub(crate) fn resolve_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        if segment.is_empty() {
            return None;
        }
        current = match current {
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            Value::Object(fields) => fields.get(segment)?,
            _ => return None,
        };
    }
    Some(current)
}

/// A JSON number the way a person would write it: integers without a
/// fraction, floats in their shortest form.
pub(crate) fn number_to_string(number: &Number) -> String {
    if let Some(value) = number.as_i64() {
        value.to_string()
    } else if let Some(value) = number.as_u64() {
        value.to_string()
    } else {
        number.as_f64().unwrap_or(f64::NAN).to_string()
    }
}

/// The item list of one poll payload. An explicit path must resolve to an
/// array; without a path an array payload is the list and any other payload
/// is a single item.
pub fn extract_poll_items(payload: &Value, items_path: Option<&str>) -> Result<Vec<Value>, String> {
    let Some(path) = items_path.filter(|path| !path.is_empty()) else {
        return Ok(match payload {
            Value::Array(items) => items.clone(),
            other => vec![other.clone()],
        });
    };
    match resolve_path(payload, path) {
        None => Err(format!("poll items path {path:?} not found in the payload")),
        Some(Value::Array(items)) => Ok(items.clone()),
        Some(_) => Err(format!("poll items path {path:?} is not an array")),
    }
}

/// Stable identity for one item under the trigger's cursor discipline:
/// the id field as a string (scalars only) or the watermark field (strings
/// and numbers only).
pub fn poll_item_key(item: &Value, cursor: &PollCursorSpec) -> Option<String> {
    match cursor {
        PollCursorSpec::IdSet { id } => match resolve_path(item, id)? {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) => Some(number_to_string(value)),
            Value::Bool(value) => Some(value.to_string()),
            Value::Null | Value::Array(_) | Value::Object(_) => None,
        },
        PollCursorSpec::Watermark { field } => watermark_value(item, field).map(watermark_string),
    }
}

fn watermark_value<'a>(item: &'a Value, field: &str) -> Option<&'a Value> {
    resolve_path(item, field).filter(|value| value.is_string() || value.is_number())
}

fn watermark_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => number_to_string(value),
        other => other.to_string(),
    }
}

/// Watermarks compare numerically when both sides are numbers, else
/// lexically (ISO-8601 timestamps compare correctly as strings).
fn watermark_after(value: &Value, mark: &Value) -> bool {
    match (value.as_f64(), mark.as_f64()) {
        (Some(value), Some(mark)) => value > mark,
        _ => watermark_string(value) > watermark_string(mark),
    }
}

fn watermark_ordering(a: &Value, b: &Value) -> std::cmp::Ordering {
    if watermark_after(a, b) {
        std::cmp::Ordering::Greater
    } else if watermark_after(b, a) {
        std::cmp::Ordering::Less
    } else {
        std::cmp::Ordering::Equal
    }
}

/// One newly seen item and its cursor key (what `poll_event_id` takes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PollItem {
    pub key: String,
    pub item: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PollDiff {
    /// First contact: the cursor was initialized and nothing is delivered.
    pub baselined: bool,
    /// Unseen items in payload order, at most [`MAX_POLL_ITEMS_PER_FIRE`].
    pub new_items: Vec<PollItem>,
    /// The cursor to store after the fire.
    pub next_state: PollCursorState,
}

/// Diff one payload against the cursor. `None` is the baseline poll: the
/// cursor initializes from the current payload and nothing delivers. A
/// successful poll clears the failure streak.
pub fn diff_poll_items(
    items: &[Value],
    cursor: &PollCursorSpec,
    state: Option<&PollCursorState>,
    now_ms: i64,
) -> PollDiff {
    match cursor {
        PollCursorSpec::IdSet { .. } => diff_id_set(items, cursor, state, now_ms),
        PollCursorSpec::Watermark { field } => diff_watermark(items, field, state, now_ms),
    }
}

fn diff_id_set(
    items: &[Value],
    cursor: &PollCursorSpec,
    state: Option<&PollCursorState>,
    now_ms: i64,
) -> PollDiff {
    let keyed: Vec<PollItem> = items
        .iter()
        .filter_map(|item| {
            poll_item_key(item, cursor).map(|key| PollItem {
                key,
                item: item.clone(),
            })
        })
        .collect();
    let Some(state) = state else {
        return PollDiff {
            baselined: true,
            new_items: Vec::new(),
            next_state: PollCursorState {
                ids: dedupe_tail(keyed.into_iter().map(|entry| entry.key)),
                watermark: None,
                consecutive_failures: 0,
                baselined_at_ms: Some(now_ms),
                last_polled_at_ms: Some(now_ms),
            },
        };
    };
    let mut seen: HashSet<&str> = state.ids.iter().map(String::as_str).collect();
    let mut fresh = Vec::new();
    for entry in &keyed {
        if fresh.len() >= MAX_POLL_ITEMS_PER_FIRE {
            break;
        }
        if seen.insert(entry.key.as_str()) {
            fresh.push(entry.clone());
        }
    }
    let ids = dedupe_tail(
        state
            .ids
            .iter()
            .cloned()
            .chain(fresh.iter().map(|entry| entry.key.clone())),
    );
    PollDiff {
        baselined: false,
        new_items: fresh,
        next_state: PollCursorState {
            ids,
            consecutive_failures: 0,
            last_polled_at_ms: Some(now_ms),
            ..state.clone()
        },
    }
}

fn diff_watermark(
    items: &[Value],
    field: &str,
    state: Option<&PollCursorState>,
    now_ms: i64,
) -> PollDiff {
    let marked: Vec<(&Value, &Value)> = items
        .iter()
        .filter_map(|item| watermark_value(item, field).map(|value| (item, value)))
        .collect();
    let mark = state.and_then(|state| state.watermark.as_ref());
    let Some((state, mark)) = state.zip(mark) else {
        let highest = marked.iter().map(|(_, value)| *value).reduce(|max, value| {
            if watermark_after(value, max) {
                value
            } else {
                max
            }
        });
        return PollDiff {
            baselined: true,
            new_items: Vec::new(),
            next_state: PollCursorState {
                ids: Vec::new(),
                watermark: highest.cloned(),
                consecutive_failures: 0,
                baselined_at_ms: Some(now_ms),
                last_polled_at_ms: Some(now_ms),
            },
        };
    };

    let mut fresh: Vec<(&Value, &Value)> = marked
        .into_iter()
        .filter(|(_, value)| watermark_after(value, mark))
        .collect();
    if fresh.len() > MAX_POLL_ITEMS_PER_FIRE {
        // Deliver the lowest marks first (ties at the boundary included, so
        // nothing hides behind the advanced watermark); the rest are above
        // the new watermark and come with the next fire.
        let mut ordered: Vec<&Value> = fresh.iter().map(|(_, value)| *value).collect();
        ordered.sort_by(|a, b| watermark_ordering(a, b));
        let threshold = ordered[MAX_POLL_ITEMS_PER_FIRE - 1];
        fresh.retain(|(_, value)| !watermark_after(value, threshold));
    }
    let highest = fresh.iter().map(|(_, value)| *value).reduce(|max, value| {
        if watermark_after(value, max) {
            value
        } else {
            max
        }
    });
    PollDiff {
        baselined: false,
        new_items: fresh
            .iter()
            .map(|(item, value)| PollItem {
                key: watermark_string(value),
                item: (*item).clone(),
            })
            .collect(),
        next_state: PollCursorState {
            watermark: Some(highest.cloned().unwrap_or_else(|| mark.clone())),
            consecutive_failures: 0,
            last_polled_at_ms: Some(now_ms),
            ..state.clone()
        },
    }
}

/// Parse a poll payload, failing with a snippet of the offending text:
/// "not JSON" debugging starts with seeing what the source produced (an
/// HTML error page, a stray warning line before the JSON). `label` names
/// the source (`stdout`, `response body`).
pub fn parse_poll_payload(bytes: &[u8], label: &str) -> Result<Value, String> {
    serde_json::from_slice(bytes).map_err(|error| {
        let text = String::from_utf8_lossy(bytes);
        let preview = text
            .chars()
            .take(PAYLOAD_PREVIEW_CHARS)
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let preview = if preview.is_empty() {
            "(empty)".to_owned()
        } else {
            preview
        };
        format!("poll {label} is not JSON ({error}); {label} starts: {preview}")
    })
}

/// One line describing an item, preferring its human-ish fields
/// (`summary`, `title`, `name`, `subject`, `message`) over its key.
pub fn poll_item_summary(item: &Value, key: &str) -> String {
    if let Value::Object(fields) = item {
        for field in SUMMARY_FIELDS {
            if let Some(text) = fields.get(field).and_then(Value::as_str) {
                let text = text.trim();
                if !text.is_empty() {
                    return truncate_chars(text, MAX_SUMMARY_CHARS).to_owned();
                }
            }
        }
    }
    format!("new item {}", truncate_chars(key, MAX_SUMMARY_KEY_CHARS))
}

/// Keep the last occurrence of every id, in order, capped to the newest
/// [`MAX_POLL_CURSOR_IDS`].
fn dedupe_tail(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    let ids: Vec<String> = ids.into_iter().collect();
    let mut seen = HashSet::new();
    let mut unique: Vec<String> = ids
        .into_iter()
        .rev()
        .filter(|id| seen.insert(id.clone()))
        .collect();
    unique.reverse();
    let excess = unique.len().saturating_sub(MAX_POLL_CURSOR_IDS);
    unique.drain(..excess);
    unique
}

fn truncate_chars(value: &str, max_chars: usize) -> &str {
    match value.char_indices().nth(max_chars) {
        Some((index, _)) => &value[..index],
        None => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_787_918_400_000;

    fn id_set() -> PollCursorSpec {
        PollCursorSpec::IdSet {
            id: "id".to_owned(),
        }
    }

    fn watermark() -> PollCursorSpec {
        PollCursorSpec::Watermark {
            field: "updatedAt".to_owned(),
        }
    }

    fn keys(diff: &PollDiff) -> Vec<&str> {
        diff.new_items
            .iter()
            .map(|entry| entry.key.as_str())
            .collect()
    }

    #[test]
    fn resolves_the_item_array_by_path_or_treats_the_payload_as_the_list() {
        assert_eq!(
            extract_poll_items(
                &json!({ "data": { "issues": [1, 2] } }),
                Some("data.issues")
            )
            .unwrap(),
            vec![json!(1), json!(2)]
        );
        assert_eq!(
            extract_poll_items(&json!([1, 2]), None).unwrap(),
            vec![json!(1), json!(2)]
        );
        assert_eq!(
            extract_poll_items(&json!({ "one": 1 }), Some("")).unwrap(),
            vec![json!({ "one": 1 })]
        );
        let missing = extract_poll_items(&json!({ "data": {} }), Some("data.issues")).unwrap_err();
        assert!(missing.contains("not found"), "{missing}");
        let scalar = extract_poll_items(&json!({ "data": { "issues": 3 } }), Some("data.issues"))
            .unwrap_err();
        assert!(scalar.contains("not an array"), "{scalar}");
        let null = extract_poll_items(&json!({ "data": { "issues": null } }), Some("data.issues"))
            .unwrap_err();
        assert!(null.contains("not an array"), "{null}");
    }

    #[test]
    fn resolve_path_walks_objects_and_arrays() {
        let value = json!({ "a": { "b": [ { "c": 1 }, null ] }, "": 2 });
        assert_eq!(resolve_path(&value, "a.b.0.c"), Some(&json!(1)));
        assert_eq!(resolve_path(&value, "a.b.1"), Some(&Value::Null));
        assert_eq!(resolve_path(&value, "a.b.2"), None);
        assert_eq!(resolve_path(&value, "a.b.x"), None);
        assert_eq!(resolve_path(&value, "a.b.0.c.d"), None);
        assert_eq!(resolve_path(&value, "a..b"), None);
        assert_eq!(resolve_path(&value, "a.b.-1"), None);
    }

    #[test]
    fn id_set_baselines_on_first_contact_without_delivering() {
        let diff = diff_poll_items(
            &[json!({ "id": 1 }), json!({ "id": 2 })],
            &id_set(),
            None,
            NOW,
        );
        assert!(diff.baselined);
        assert!(diff.new_items.is_empty());
        assert_eq!(diff.next_state.ids, vec!["1", "2"]);
        assert_eq!(diff.next_state.baselined_at_ms, Some(NOW));
        assert_eq!(diff.next_state.last_polled_at_ms, Some(NOW));
        assert_eq!(diff.next_state.consecutive_failures, 0);
        assert_eq!(diff.next_state.watermark, None);
    }

    #[test]
    fn id_set_delivers_only_unseen_ids_and_advances_the_cursor() {
        let state = PollCursorState {
            ids: vec!["1".to_owned(), "2".to_owned()],
            consecutive_failures: 3,
            ..Default::default()
        };
        let items = [json!({ "id": 2 }), json!({ "id": 3 }), json!({ "id": 3 })];
        let diff = diff_poll_items(&items, &id_set(), Some(&state), NOW);
        assert!(!diff.baselined);
        assert_eq!(keys(&diff), vec!["3"]);
        assert_eq!(diff.new_items[0].item, json!({ "id": 3 }));
        assert_eq!(diff.next_state.ids, vec!["1", "2", "3"]);
        // A successful poll clears the failure streak.
        assert_eq!(diff.next_state.consecutive_failures, 0);
        assert_eq!(diff.next_state.last_polled_at_ms, Some(NOW));
        assert_eq!(diff.next_state.baselined_at_ms, None);
    }

    #[test]
    fn id_set_caps_the_ids_aging_out_the_oldest() {
        let state = PollCursorState {
            ids: (0..MAX_POLL_CURSOR_IDS)
                .map(|index| format!("old-{index}"))
                .collect(),
            ..Default::default()
        };
        let diff = diff_poll_items(&[json!({ "id": "fresh" })], &id_set(), Some(&state), NOW);
        assert_eq!(diff.next_state.ids.len(), MAX_POLL_CURSOR_IDS);
        assert_eq!(
            diff.next_state.ids.last().map(String::as_str),
            Some("fresh")
        );
        assert!(!diff.next_state.ids.iter().any(|id| id == "old-0"));
        assert!(diff.next_state.ids.iter().any(|id| id == "old-1"));
    }

    #[test]
    fn id_set_skips_items_without_a_usable_id() {
        let state = PollCursorState::default();
        let items = [
            json!({ "id": null }),
            json!({ "x": 1 }),
            json!({ "id": { "nested": 1 } }),
            json!({ "id": [1] }),
            json!("scalar"),
        ];
        let diff = diff_poll_items(&items, &id_set(), Some(&state), NOW);
        assert!(!diff.baselined);
        assert!(diff.new_items.is_empty());
        assert!(diff.next_state.ids.is_empty());
    }

    #[test]
    fn id_set_delivers_at_most_the_per_fire_cap_and_keeps_the_rest_for_the_next_fire() {
        let items: Vec<Value> = (0..150).map(|index| json!({ "id": index })).collect();
        let state = PollCursorState::default();
        let first = diff_poll_items(&items, &id_set(), Some(&state), NOW);
        assert_eq!(first.new_items.len(), MAX_POLL_ITEMS_PER_FIRE);
        assert_eq!(keys(&first)[0], "0");
        assert_eq!(keys(&first)[MAX_POLL_ITEMS_PER_FIRE - 1], "99");
        assert_eq!(first.next_state.ids.len(), MAX_POLL_ITEMS_PER_FIRE);

        let second = diff_poll_items(&items, &id_set(), Some(&first.next_state), NOW + 1);
        assert_eq!(second.new_items.len(), 50);
        assert_eq!(keys(&second)[0], "100");
        assert_eq!(keys(&second)[49], "149");
        assert_eq!(second.next_state.ids.len(), 150);

        let third = diff_poll_items(&items, &id_set(), Some(&second.next_state), NOW + 2);
        assert!(third.new_items.is_empty());
    }

    #[test]
    fn watermark_baselines_to_the_highest_mark_then_delivers_only_newer_items() {
        let first = diff_poll_items(
            &[
                json!({ "updatedAt": "2026-01-01" }),
                json!({ "updatedAt": "2026-03-01" }),
            ],
            &watermark(),
            None,
            NOW,
        );
        assert!(first.baselined);
        assert!(first.new_items.is_empty());
        assert_eq!(first.next_state.watermark, Some(json!("2026-03-01")));
        assert_eq!(first.next_state.baselined_at_ms, Some(NOW));

        let second = diff_poll_items(
            &[
                json!({ "updatedAt": "2026-02-01" }),
                json!({ "updatedAt": "2026-04-01" }),
            ],
            &watermark(),
            Some(&first.next_state),
            NOW + 1,
        );
        assert!(!second.baselined);
        assert_eq!(keys(&second), vec!["2026-04-01"]);
        assert_eq!(second.next_state.watermark, Some(json!("2026-04-01")));
        assert_eq!(second.next_state.baselined_at_ms, Some(NOW));
        assert_eq!(second.next_state.last_polled_at_ms, Some(NOW + 1));

        // Nothing newer keeps the watermark where it is.
        let third = diff_poll_items(
            &[json!({ "updatedAt": "2026-01-01" })],
            &watermark(),
            Some(&second.next_state),
            NOW + 2,
        );
        assert!(third.new_items.is_empty());
        assert_eq!(third.next_state.watermark, Some(json!("2026-04-01")));
    }

    #[test]
    fn watermark_state_without_a_mark_is_a_baseline() {
        let state = PollCursorState {
            consecutive_failures: 2,
            ..Default::default()
        };
        let diff = diff_poll_items(
            &[json!({ "updatedAt": 5 })],
            &watermark(),
            Some(&state),
            NOW,
        );
        assert!(diff.baselined);
        assert_eq!(diff.next_state.watermark, Some(json!(5)));
        assert_eq!(diff.next_state.consecutive_failures, 0);

        // An empty payload baselines without a mark; the next poll baselines
        // again.
        let empty = diff_poll_items(&[], &watermark(), None, NOW);
        assert!(empty.baselined);
        assert_eq!(empty.next_state.watermark, None);
    }

    #[test]
    fn compares_numeric_watermarks_numerically() {
        let state = PollCursorState {
            watermark: Some(json!(90)),
            ..Default::default()
        };
        let diff = diff_poll_items(
            &[json!({ "updatedAt": 100 }), json!({ "updatedAt": 9 })],
            &watermark(),
            Some(&state),
            NOW,
        );
        assert_eq!(keys(&diff), vec!["100"]);
        assert_eq!(diff.next_state.watermark, Some(json!(100)));

        // Lexically "9" > "100"; numerically it is not.
        assert!(!watermark_after(&json!(9), &json!(100)));
        assert!(watermark_after(&json!(100), &json!(9)));
        assert!(watermark_after(&json!("9"), &json!("100")));
        assert!(watermark_after(&json!(10.5), &json!(10)));
        // Mixed types fall back to the lexical comparison.
        assert!(watermark_after(&json!("b"), &json!(1)));
    }

    #[test]
    fn watermark_delivers_the_lowest_marks_up_to_the_cap_with_ties() {
        let mut items: Vec<Value> = (0..150)
            .map(|index| json!({ "updatedAt": index + 1 }))
            .collect();
        // A tie exactly at the boundary must ride along.
        items.push(json!({ "updatedAt": 100, "dup": true }));
        // The payload is unordered.
        items.reverse();
        let state = PollCursorState {
            watermark: Some(json!(0)),
            ..Default::default()
        };
        let first = diff_poll_items(&items, &watermark(), Some(&state), NOW);
        assert_eq!(first.new_items.len(), MAX_POLL_ITEMS_PER_FIRE + 1);
        assert!(
            first
                .new_items
                .iter()
                .all(|entry| entry.item["updatedAt"].as_i64().unwrap() <= 100)
        );
        assert!(
            first
                .new_items
                .iter()
                .any(|entry| entry.item.get("dup").is_some())
        );
        assert_eq!(first.next_state.watermark, Some(json!(100)));

        let second = diff_poll_items(&items, &watermark(), Some(&first.next_state), NOW + 1);
        assert_eq!(second.new_items.len(), 50);
        assert!(
            second
                .new_items
                .iter()
                .all(|entry| entry.item["updatedAt"].as_i64().unwrap() > 100)
        );
        assert_eq!(second.next_state.watermark, Some(json!(150)));
    }

    #[test]
    fn derives_keys_from_nested_paths() {
        assert_eq!(
            poll_item_key(
                &json!({ "issue": { "number": 7 } }),
                &PollCursorSpec::IdSet {
                    id: "issue.number".to_owned()
                }
            )
            .as_deref(),
            Some("7")
        );
        assert_eq!(
            poll_item_key(
                &json!({ "x": {} }),
                &PollCursorSpec::IdSet { id: "x".to_owned() }
            ),
            None
        );
        assert_eq!(
            poll_item_key(&json!({ "id": true }), &id_set()).as_deref(),
            Some("true")
        );
        assert_eq!(
            poll_item_key(&json!({ "id": 2.5 }), &id_set()).as_deref(),
            Some("2.5")
        );
        assert_eq!(
            poll_item_key(&json!({ "updatedAt": "2026-01-01" }), &watermark()).as_deref(),
            Some("2026-01-01")
        );
        assert_eq!(
            poll_item_key(&json!({ "updatedAt": true }), &watermark()),
            None
        );
        assert_eq!(
            poll_item_key(&json!({ "updatedAt": null }), &watermark()),
            None
        );
    }

    #[test]
    fn prefers_human_fields_in_summaries() {
        assert_eq!(
            poll_item_summary(&json!({ "title": "Broken build" }), "77"),
            "Broken build"
        );
        assert_eq!(
            poll_item_summary(&json!({ "name": "  padded  ", "title": "" }), "77"),
            "padded"
        );
        assert_eq!(
            poll_item_summary(&json!({ "summary": "s", "title": "t" }), "77"),
            "s"
        );
        assert_eq!(poll_item_summary(&json!({ "id": 77 }), "77"), "new item 77");
        assert_eq!(poll_item_summary(&json!([1]), "k"), "new item k");
        let long = "x".repeat(500);
        assert_eq!(
            poll_item_summary(&json!({ "message": long }), "k")
                .chars()
                .count(),
            MAX_SUMMARY_CHARS
        );
        let long_key = "k".repeat(200);
        assert_eq!(
            poll_item_summary(&json!({}), &long_key),
            format!("new item {}", "k".repeat(MAX_SUMMARY_KEY_CHARS))
        );
    }

    #[test]
    fn parses_json_and_shows_the_offending_text_otherwise() {
        assert_eq!(
            parse_poll_payload(br#"{"a":1}"#, "stdout").unwrap(),
            json!({ "a": 1 })
        );
        let warning = parse_poll_payload(b"Warning: deprecated\n[1,2]", "stdout").unwrap_err();
        assert!(warning.contains("poll stdout is not JSON"), "{warning}");
        assert!(
            warning.ends_with("stdout starts: Warning: deprecated [1,2]"),
            "{warning}"
        );
        let html = parse_poll_payload(b"<html>login</html>", "response body").unwrap_err();
        assert!(
            html.ends_with("response body starts: <html>login</html>"),
            "{html}"
        );
        let empty = parse_poll_payload(b"", "stdout").unwrap_err();
        assert!(empty.ends_with("(empty)"), "{empty}");
        let long = vec![b'x'; 1_000];
        let capped = parse_poll_payload(&long, "stdout").unwrap_err();
        assert!(
            capped.ends_with(&"x".repeat(PAYLOAD_PREVIEW_CHARS)),
            "{capped}"
        );
        assert!(!capped.ends_with(&"x".repeat(PAYLOAD_PREVIEW_CHARS + 1)));
    }

    #[test]
    fn dedupe_tail_keeps_the_last_occurrence_in_order() {
        let ids = ["a", "b", "a", "c"].map(str::to_owned);
        assert_eq!(dedupe_tail(ids), vec!["b", "a", "c"]);
        let many: Vec<String> = (0..(MAX_POLL_CURSOR_IDS + 10))
            .map(|i| i.to_string())
            .collect();
        let capped = dedupe_tail(many);
        assert_eq!(capped.len(), MAX_POLL_CURSOR_IDS);
        assert_eq!(capped[0], "10");
    }
}
