//! Build identity embedded in every released Lightspeed executable.

pub const VERSION: &str = env!("LIGHTSPEED_BUILD_VERSION");
pub const GIT_SHA: &str = env!("LIGHTSPEED_BUILD_GIT_SHA");
pub const RUST_VERSION: &str = env!("LIGHTSPEED_BUILD_RUST_VERSION");
pub const TARGET: &str = env!("LIGHTSPEED_BUILD_TARGET");
/// Rust targets the environment daemon is published for in the release this
/// binary belongs to, comma separated. A local build names its own target.
pub const ENVD_TARGETS: &str = env!("LIGHTSPEED_BUILD_ENVD_TARGETS");

pub fn envd_targets() -> impl Iterator<Item = &'static str> {
    ENVD_TARGETS
        .split(',')
        .map(str::trim)
        .filter(|target| !target.is_empty())
}
pub const LONG_VERSION: &str = concat!(
    env!("LIGHTSPEED_BUILD_VERSION"),
    " (git ",
    env!("LIGHTSPEED_BUILD_GIT_SHA"),
    ", ",
    env!("LIGHTSPEED_BUILD_TARGET"),
    ", ",
    env!("LIGHTSPEED_BUILD_RUST_VERSION"),
    ")"
);
