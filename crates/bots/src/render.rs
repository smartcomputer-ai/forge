//! Model-facing event rendering. The stored [`BotEventDocument`] stays the
//! complete machine envelope (filters, UI, replay, `bot_event_read`); what a
//! session reads is a compact text produced here. Pruning is by shape, never
//! by service knowledge, and every cut is marked so the model knows to pull
//! the full payload with `bot_event_read`.

use api::BotEventDocument;
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::Value;

/// Keys that carry an object's human identity, used to collapse it.
const NAME_KEYS: [&str; 6] = ["login", "full_name", "name", "slug", "username", "email"];
/// Keys tolerated (and hidden) when collapsing an identity object.
const IDENTITY_NOISE: [&str; 4] = ["id", "type", "site_admin", "user_view_type"];
/// Key suffixes (whole key, or after an underscore) whose values are API
/// plumbing, dropped at any depth.
const DROP_SUFFIXES: [&str; 5] = ["url", "urls", "href", "link", "links"];
/// Exact keys dropped at any depth.
const DROP_EXACT: [&str; 4] = ["node_id", "gravatar_id", "etag", "_links"];

pub const MAX_STRING: usize = 400;
pub const MAX_ARRAY_ITEMS: usize = 6;
pub const MAX_DEPTH: usize = 6;
pub const DEFAULT_PROMPT_BUDGET: usize = 2_048;
pub const DEFAULT_READ_BUDGET: usize = 8_192;

/// Whether a key's value is API plumbing: `url`, `html_url`, `_links`,
/// `node_id`, … at any depth.
pub fn is_dropped_key(key: &str) -> bool {
    if DROP_EXACT.contains(&key) {
        return true;
    }
    DROP_SUFFIXES.iter().any(|suffix| {
        key.strip_suffix(suffix)
            .is_some_and(|prefix| prefix.is_empty() || prefix.ends_with('_'))
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderedValue {
    pub text: String,
    /// True when anything was dropped, truncated, or capped.
    pub elided: bool,
}

struct RenderState {
    lines: Vec<String>,
    bytes: usize,
    budget: usize,
    elided: bool,
    overflowed: bool,
}

impl RenderState {
    fn emit(&mut self, line: String) -> bool {
        if self.overflowed {
            return false;
        }
        if self.bytes + line.len() + 1 > self.budget {
            self.overflowed = true;
            return false;
        }
        self.bytes += line.len() + 1;
        self.lines.push(line);
        true
    }
}

/// `YYYY-MM-DD HH:MMZ` for the event header; out-of-range instants fall
/// back to the raw millisecond value.
pub fn compact_time(occurred_at_ms: i64) -> String {
    match DateTime::<Utc>::from_timestamp_millis(occurred_at_ms) {
        Some(instant) => instant.format("%Y-%m-%d %H:%MZ").to_string(),
        None => occurred_at_ms.to_string(),
    }
}

/// RFC 3339 with milliseconds (`2026-08-26T10:00:00.000Z`) for model-facing
/// timestamps; out-of-range instants fall back to the raw value.
pub fn iso_time(at_ms: i64) -> String {
    match DateTime::<Utc>::from_timestamp_millis(at_ms) {
        Some(instant) => instant.to_rfc3339_opts(SecondsFormat::Millis, true),
        None => at_ms.to_string(),
    }
}

/// Render the delivered representation of one event: header, summary,
/// pruned payload, correlation, links, and an honest footer when anything
/// was cut. `prompt_data` (a preset's salient projection) renders instead of
/// `document.data` when given; headers are never rendered.
pub fn render_event_prompt(
    seq: u64,
    document: &BotEventDocument,
    prompt_data: Option<&Value>,
    max_bytes: usize,
) -> String {
    let mut parts = vec![
        format!(
            "── event #{seq} · {} · {} · {}",
            document.kind,
            document.source,
            compact_time(document.occurred_at_ms)
        ),
        document.summary.clone(),
    ];
    let mut elided = false;
    let data = prompt_data.or(document.data.as_ref());
    if let Some(data) = data
        && !data.is_null()
    {
        let rendered = render_value(data, max_bytes);
        if !rendered.text.is_empty() {
            parts.push(rendered.text);
        }
        elided = rendered.elided;
    }
    if let Some(reply) = &document.in_reply_to {
        parts.push(format!("reply to your #{} at {}", reply.seq, reply.bot));
    }
    if let Some(correlation) = &document.correlation_id {
        parts.push(format!("correlation: {correlation}"));
    }
    if !document.links.is_empty() {
        let links: Vec<&str> = document.links.iter().take(5).map(String::as_str).collect();
        parts.push(format!("links: {}", links.join(" ")));
    }
    if elided {
        parts.push(format!("(… pruned — full payload: bot_event_read #{seq})"));
    }
    parts.join("\n")
}

/// Render arbitrary JSON as compact indented text with shape-based pruning,
/// stopping at `max_bytes` with an explicit truncation mark.
pub fn render_value(value: &Value, max_bytes: usize) -> RenderedValue {
    let mut state = RenderState {
        lines: Vec::new(),
        bytes: 0,
        budget: max_bytes,
        elided: false,
        overflowed: false,
    };
    render_node(value, "", 0, &mut state);
    if state.overflowed {
        state.lines.push("(truncated)".to_owned());
        state.elided = true;
    }
    RenderedValue {
        text: state.lines.join("\n"),
        elided: state.elided,
    }
}

fn render_node(value: &Value, indent: &str, depth: usize, state: &mut RenderState) {
    if let Some(scalar) = render_scalar(value, state) {
        state.emit(format!("{indent}{scalar}"));
        return;
    }
    if depth >= MAX_DEPTH {
        state.elided = true;
        state.emit(format!("{indent}…"));
        return;
    }
    match value {
        Value::Array(items) => {
            for item in items.iter().take(MAX_ARRAY_ITEMS) {
                match render_scalar(item, state) {
                    Some(inline) => {
                        state.emit(format!("{indent}- {inline}"));
                    }
                    None => {
                        state.emit(format!("{indent}-"));
                        render_node(item, &format!("{indent}  "), depth + 1, state);
                    }
                }
            }
            if items.len() > MAX_ARRAY_ITEMS {
                state.elided = true;
                state.emit(format!(
                    "{indent}… and {} more",
                    items.len() - MAX_ARRAY_ITEMS
                ));
            }
        }
        Value::Object(entries) => {
            for (key, entry) in entries {
                if state.overflowed {
                    return;
                }
                if drop_entry(key, entry) {
                    if !entry.is_null() && !is_empty_container(entry) {
                        state.elided = true;
                    }
                    continue;
                }
                if let Some(inline) = render_scalar(entry, state) {
                    state.emit(format!("{indent}{key}: {inline}"));
                    continue;
                }
                if let Some(identity) = collapse_identity(entry) {
                    state.elided = true;
                    state.emit(format!("{indent}{key}: {identity}"));
                    continue;
                }
                state.emit(format!("{indent}{key}:"));
                render_node(entry, &format!("{indent}  "), depth + 1, state);
            }
        }
        // Scalars were handled above.
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// Scalar rendering, or `None` when the value needs structural layout.
fn render_scalar(value: &Value, state: &mut RenderState) -> Option<String> {
    match value {
        Value::Null => Some("null".to_owned()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Number(number) => Some(number.to_string()),
        Value::String(text) => {
            let flattened = flatten_newlines(text);
            let length = flattened.chars().count();
            if length > MAX_STRING {
                state.elided = true;
                let head: String = flattened.chars().take(MAX_STRING).collect();
                Some(format!("{head}… (+{})", format_bytes(length - MAX_STRING)))
            } else {
                Some(flattened)
            }
        }
        Value::Array(_) | Value::Object(_) => None,
    }
}

/// Every whitespace run containing a newline becomes ` ⏎ `, so a multi-line
/// body stays one line.
fn flatten_newlines(text: &str) -> String {
    if !text.contains('\n') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut run = String::new();
    let mut run_has_newline = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            run.push(ch);
            run_has_newline |= ch == '\n';
            continue;
        }
        if run_has_newline {
            out.push_str(" ⏎ ");
        } else {
            out.push_str(&run);
        }
        run.clear();
        run_has_newline = false;
        out.push(ch);
    }
    if run_has_newline {
        out.push_str(" ⏎ ");
    } else {
        out.push_str(&run);
    }
    out
}

fn drop_entry(key: &str, value: &Value) -> bool {
    value.is_null() || is_dropped_key(key) || is_empty_container(value)
}

fn is_empty_container(value: &Value) -> bool {
    match value {
        Value::Array(items) => items.is_empty(),
        Value::Object(entries) => entries.is_empty(),
        _ => false,
    }
}

/// An object that is just an identity (a name key plus ids and urls) renders
/// as its name: `user: lukas` instead of eight lines of avatar plumbing.
pub fn collapse_identity(value: &Value) -> Option<String> {
    let Value::Object(entries) = value else {
        return None;
    };
    let name = NAME_KEYS.iter().find_map(|key| match entries.get(*key) {
        Some(Value::String(candidate)) if !candidate.is_empty() => Some(candidate.clone()),
        _ => None,
    })?;
    for (key, entry) in entries {
        if NAME_KEYS.contains(&key.as_str()) {
            continue;
        }
        if IDENTITY_NOISE.contains(&key.as_str()) || is_dropped_key(key) {
            continue;
        }
        if entry.is_null() || is_empty_container(entry) {
            continue;
        }
        return None;
    }
    Some(name)
}

/// Walk a dot path (array indices as numbers: `commits.0.message`).
pub fn resolve_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        if segment.is_empty() {
            return None;
        }
        current = match current {
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            Value::Object(entries) => entries.get(segment)?,
            _ => return None,
        };
    }
    Some(current)
}

/// One child branch of a value, sized by its JSON encoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LargestBranch {
    pub path: String,
    pub bytes: usize,
    /// Array children: their length.
    pub items: Option<usize>,
}

impl LargestBranch {
    pub fn to_value(&self) -> Value {
        let mut object = serde_json::Map::new();
        object.insert("path".to_owned(), Value::from(self.path.clone()));
        object.insert("bytes".to_owned(), Value::from(self.bytes));
        if let Some(items) = self.items {
            object.insert("items".to_owned(), Value::from(items));
        }
        Value::Object(object)
    }
}

/// Largest child branches of a value, for honest over-budget reporting.
pub fn largest_branches(value: &Value, limit: usize) -> Vec<LargestBranch> {
    let entries: Vec<(String, &Value)> = match value {
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, item)| (index.to_string(), item))
            .collect(),
        Value::Object(entries) => entries
            .iter()
            .map(|(key, entry)| (key.clone(), entry))
            .collect(),
        _ => return Vec::new(),
    };
    let mut branches: Vec<LargestBranch> = entries
        .into_iter()
        .map(|(path, entry)| LargestBranch {
            path,
            bytes: serde_json::to_string(entry).map_or(0, |json| json.len()),
            items: match entry {
                Value::Array(items) => Some(items.len()),
                _ => None,
            },
        })
        .collect();
    branches.sort_by_key(|branch| std::cmp::Reverse(branch.bytes));
    branches.truncate(limit);
    branches
}

pub fn format_bytes(bytes: usize) -> String {
    if bytes < 1_024 {
        format!("{bytes} B")
    } else {
        format!("{:.1} KB", bytes as f64 / 1_024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{BotEventReplyRef, BotId};
    use serde_json::json;

    fn document(kind: &str, source: &str, occurred_at_ms: i64, summary: &str) -> BotEventDocument {
        BotEventDocument {
            version: BotEventDocument::VERSION,
            kind: kind.to_owned(),
            source: source.to_owned(),
            occurred_at_ms,
            summary: summary.to_owned(),
            data: None,
            headers: Default::default(),
            correlation_id: None,
            links: Vec::new(),
            sender: None,
            hops: 0,
            in_reply_to: None,
        }
    }

    #[test]
    fn drop_keys_match_the_plumbing_shapes() {
        for key in [
            "url",
            "urls",
            "href",
            "link",
            "links",
            "html_url",
            "avatar_urls",
            "self_href",
            "next_link",
            "_links",
            "node_id",
            "gravatar_id",
            "etag",
        ] {
            assert!(is_dropped_key(key), "{key} should be dropped");
        }
        for key in [
            "title",
            "curl",
            "urlencoded",
            "linkage",
            "hyperlink_text",
            "id",
            "etags",
        ] {
            assert!(!is_dropped_key(key), "{key} should be kept");
        }
    }

    #[test]
    fn drops_plumbing_by_shape() {
        let rendered = render_value(
            &json!({
                "title": "Fix rate limiter",
                "html_url": "https://example.com/pr/877",
                "_links": { "self": { "href": "x" } },
                "node_id": "MDEx",
                "labels": [],
                "assignee": null,
                "draft": false,
                "number": 877
            }),
            DEFAULT_PROMPT_BUDGET,
        );
        assert!(
            rendered.text.contains("title: Fix rate limiter"),
            "{}",
            rendered.text
        );
        assert!(rendered.text.contains("draft: false"));
        assert!(rendered.text.contains("number: 877"));
        assert!(!rendered.text.contains("html_url"));
        assert!(!rendered.text.contains("node_id"));
        assert!(!rendered.text.contains("labels"));
        assert!(rendered.elided);
    }

    #[test]
    fn collapses_identity_objects_to_their_name() {
        let rendered = render_value(
            &json!({
                "user": { "login": "lukas", "id": 7, "avatar_url": "https://a", "type": "User", "site_admin": false },
                "base": { "ref": "main", "sha": "abc" }
            }),
            DEFAULT_PROMPT_BUDGET,
        );
        assert!(rendered.text.contains("user: lukas"), "{}", rendered.text);
        assert!(!rendered.text.contains("avatar"));
        // Objects with substantive extra fields keep their structure.
        assert!(rendered.text.contains("base:"));
        assert!(rendered.text.contains("  ref: main"));
        assert_eq!(
            collapse_identity(&json!({ "name": "octo", "url": "https://x", "id": 1 })).as_deref(),
            Some("octo")
        );
        assert_eq!(
            collapse_identity(&json!({ "name": "octo", "role": "admin" })),
            None
        );
        assert_eq!(collapse_identity(&json!({ "id": 1 })), None);
        assert_eq!(collapse_identity(&json!(["octo"])), None);
    }

    #[test]
    fn truncates_long_strings_and_caps_arrays_with_visible_marks() {
        let commits: Vec<String> = (0..10).map(|index| format!("c{index}")).collect();
        let rendered = render_value(
            &json!({ "body": "x".repeat(1_000), "commits": commits }),
            DEFAULT_PROMPT_BUDGET,
        );
        let expected = format!("{}… (+600 B)", "x".repeat(400));
        assert!(rendered.text.contains(&expected), "{}", rendered.text);
        assert!(!rendered.text.contains(&"x".repeat(401)));
        assert!(rendered.text.contains("- c5"));
        assert!(!rendered.text.contains("- c6"));
        assert!(rendered.text.contains("… and 4 more"));
        assert!(rendered.elided);
    }

    #[test]
    fn flattens_multi_line_strings() {
        let rendered = render_value(&json!({ "body": "line one \n\n  line two" }), 512);
        assert_eq!(rendered.text, "body: line one ⏎ line two");
        assert!(!rendered.elided);
        assert_eq!(flatten_newlines("a\tb"), "a\tb");
        assert_eq!(flatten_newlines("trail\n"), "trail ⏎ ");
    }

    #[test]
    fn stops_at_the_byte_budget_with_an_explicit_truncation_mark() {
        let wide: serde_json::Map<String, Value> = (0..200)
            .map(|index| {
                (
                    format!("key_{index:03}"),
                    Value::from(format!("value {index}")),
                )
            })
            .collect();
        let rendered = render_value(&Value::Object(wide), 300);
        assert!(rendered.text.len() < 400, "{}", rendered.text.len());
        assert!(rendered.text.ends_with("(truncated)"));
        assert!(rendered.text.contains("key_000: value 0"));
        assert!(!rendered.text.contains("key_199"));
        assert!(rendered.elided);
    }

    #[test]
    fn nests_arrays_of_objects_and_stops_at_max_depth() {
        let rendered = render_value(
            &json!({ "commits": [{ "message": "fix", "author": { "name": "lukas", "email": "l@x" } }] }),
            DEFAULT_PROMPT_BUDGET,
        );
        assert_eq!(
            rendered.text,
            "commits:\n  -\n    author: lukas\n    message: fix"
        );
        assert!(rendered.elided, "collapsing an identity is a cut");

        let mut deep = json!("leaf");
        for _ in 0..8 {
            deep = json!({ "n": deep });
        }
        let rendered = render_value(&deep, DEFAULT_PROMPT_BUDGET);
        assert!(rendered.text.contains('…'), "{}", rendered.text);
        assert!(!rendered.text.contains("leaf"));
        assert!(rendered.elided);
    }

    #[test]
    fn renders_a_compact_schedule_event_with_a_seq_header_and_no_footer() {
        let mut document = document(
            "schedule",
            "schedule:daily-report",
            1_787_562_000_000,
            "Daily report",
        );
        document.data =
            Some(json!({ "trigger": "daily-report", "cron": "0 9 * * *", "timezone": "UTC" }));
        let prompt = render_event_prompt(142, &document, None, DEFAULT_PROMPT_BUDGET);
        assert!(
            prompt.starts_with("── event #142 · schedule · schedule:daily-report · 2026-08-24 09:00Z\nDaily report\n"),
            "{prompt}"
        );
        assert!(prompt.contains("cron: 0 9 * * *"));
        assert!(!prompt.contains("pruned"));
    }

    #[test]
    fn compact_time_formats_utc_minutes() {
        assert_eq!(compact_time(1_787_562_000_000), "2026-08-24 09:00Z");
        assert_eq!(iso_time(1_787_562_000_000), "2026-08-24T09:00:00.000Z");
        assert_eq!(iso_time(1_787_562_000_123), "2026-08-24T09:00:00.123Z");
        assert_eq!(compact_time(0), "1970-01-01 00:00Z");
    }

    #[test]
    fn points_pruned_events_at_bot_event_read_by_number() {
        let mut document = document(
            "pull_request.opened",
            "webhook:gh",
            0,
            "GitHub pull_request.opened in acme/api",
        );
        document.data = Some(json!({ "pull_request": { "title": "t", "html_url": "https://x" } }));
        let prompt = render_event_prompt(9, &document, None, DEFAULT_PROMPT_BUDGET);
        assert!(
            prompt.contains("(… pruned — full payload: bot_event_read #9)"),
            "{prompt}"
        );
        assert!(!prompt.contains("https://x"));
    }

    #[test]
    fn prompt_data_overrides_data_and_headers_never_render() {
        let mut document = document("push", "webhook:gh", 0, "push");
        document.data = Some(json!({ "raw": "everything" }));
        document.headers = [("x-github-event".to_owned(), "push".to_owned())]
            .into_iter()
            .collect();
        let projected = json!({ "salient": "only this" });
        let prompt = render_event_prompt(3, &document, Some(&projected), DEFAULT_PROMPT_BUDGET);
        assert!(prompt.contains("salient: only this"), "{prompt}");
        assert!(!prompt.contains("everything"));
        assert!(!prompt.contains("x-github-event"));
        let plain = render_event_prompt(3, &document, None, DEFAULT_PROMPT_BUDGET);
        assert!(plain.contains("raw: everything"));
    }

    #[test]
    fn renders_reply_correlation_and_links() {
        let mut document = document("bot.reply", "bot:infra", 0, "root cause: bad deploy");
        document.data = Some(json!({ "status": "handled" }));
        document.in_reply_to = Some(BotEventReplyRef {
            bot: BotId::new("infra"),
            seq: 17,
        });
        document.correlation_id = Some("inc-7".to_owned());
        document.links = (0..7).map(|index| format!("https://l/{index}")).collect();
        let prompt = render_event_prompt(12, &document, None, DEFAULT_PROMPT_BUDGET);
        assert!(
            prompt.contains("event #12 · bot.reply · bot:infra"),
            "{prompt}"
        );
        assert!(prompt.contains("status: handled"));
        assert!(prompt.contains("reply to your #17 at infra"));
        assert!(prompt.contains("correlation: inc-7"));
        assert!(
            prompt.contains("links: https://l/0 https://l/1 https://l/2 https://l/3 https://l/4\n")
                || prompt.ends_with(
                    "links: https://l/0 https://l/1 https://l/2 https://l/3 https://l/4"
                )
        );
        assert!(!prompt.contains("https://l/5"));
        assert!(!prompt.contains("pruned"));
    }

    #[test]
    fn walks_objects_and_array_indices() {
        let value = json!({ "data": { "commits": [{ "message": "fix" }, { "message": "feat" }] }, "headers": { "a": "1" } });
        assert_eq!(
            resolve_path(&value, "data.commits.1.message"),
            Some(&json!("feat"))
        );
        assert_eq!(resolve_path(&value, "headers"), Some(&json!({ "a": "1" })));
        assert_eq!(resolve_path(&value, "data.missing"), None);
        assert_eq!(resolve_path(&value, "data.commits.x"), None);
        assert_eq!(resolve_path(&value, "data.commits.7"), None);
        assert_eq!(resolve_path(&value, "data..commits"), None);
        assert_eq!(resolve_path(&value, "headers.a.b"), None);
    }

    #[test]
    fn reports_the_biggest_children_with_sizes() {
        let commits: Vec<Value> = (0..50)
            .map(|_| json!({ "message": "a commit message" }))
            .collect();
        let branches =
            largest_branches(&json!({ "commits": commits, "ref": "refs/heads/main" }), 5);
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0].path, "commits");
        assert_eq!(branches[0].items, Some(50));
        assert!(branches[0].bytes > branches[1].bytes);
        assert_eq!(branches[1].items, None);
        assert_eq!(branches[0].to_value()["items"], json!(50));
        assert!(branches[1].to_value().get("items").is_none());
        assert_eq!(
            largest_branches(&json!({ "a": 1, "b": 22, "c": 333 }), 2).len(),
            2
        );
        assert_eq!(largest_branches(&json!([1, [1, 2, 3]]), 5)[0].path, "1");
        assert!(largest_branches(&json!("scalar"), 5).is_empty());
    }

    #[test]
    fn formats_bytes() {
        assert_eq!(format_bytes(600), "600 B");
        assert_eq!(format_bytes(1_024), "1.0 KB");
        assert_eq!(format_bytes(1_536), "1.5 KB");
    }
}
