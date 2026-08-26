// Shared by the caching live suites: a prompt long enough to clear every
// provider's minimum cacheable prefix (Anthropic 1024–2048 tokens, OpenAI
// 1024) so cache reads are expected, not optional.

/// About 3k tokens of deterministic, model-neutral instructions.
#[allow(dead_code)]
pub fn long_instructions() -> String {
    let mut text = String::from(
        "You are a meticulous operations assistant. Follow every guideline below exactly and keep answers short.\n\n",
    );
    for index in 1..=240 {
        text.push_str(&format!(
            "Guideline {index}: when a request mentions item {index}, confirm the item number, note that guideline {index} applies, and never invent details that were not provided.\n"
        ));
    }
    text
}

/// Cache reads must cover most of the previous prompt; OpenAI rounds down to
/// 128-token blocks and both providers exclude the newest turn.
#[allow(dead_code)]
pub const MIN_CACHED_SHARE: f64 = 0.8;

#[allow(dead_code)]
pub fn assert_cached_share(label: &str, cached: u32, previous_input: u32) {
    let share = f64::from(cached) / f64::from(previous_input.max(1));
    eprintln!(
        "cached share [{label}]: {cached} of the previous {previous_input} prompt tokens read from cache ({:.0}%)",
        share * 100.0
    );
    assert!(
        share >= MIN_CACHED_SHARE,
        "{label}: expected at least {:.0}% of the previous prompt ({previous_input} tokens) to be read from the cache, got {cached} ({:.0}%)",
        MIN_CACHED_SHARE * 100.0,
        share * 100.0
    );
}
