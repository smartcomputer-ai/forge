// Shared by the caching live suites: a reference long enough to clear the
// tested models' minimum cacheable prefix so cache reads are required.

/// A deterministic operations reference shared by successive requests.
#[allow(dead_code)]
pub fn long_instructions() -> String {
    let mut text = String::from(
        "You are a warehouse operations assistant. Use the stock table for inventory questions and available tools for current information such as weather. Give brief, factual answers.\n\n# Warehouse stock table\n\nBay | Stock item | Units on hand | Reorder point | Inspection\n--- | --- | --- | --- | ---\n",
    );
    let items = [
        "reusable shipping containers",
        "cardboard cartons",
        "paper mailers",
        "cotton tote bags",
        "wooden pallets",
        "packing tape rolls",
    ];
    for index in 1..=240 {
        let item = items[(index - 1) % items.len()];
        let quantity = 24 + (index % 16) * 3;
        let reorder_point = 8 + index % 8;
        text.push_str(&format!(
            "{index} | {item} | {quantity} | {reorder_point} | Checked and ready for dispatch; routine maintenance complete.\n"
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
