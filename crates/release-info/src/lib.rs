//! Build identity embedded in every released Lightspeed executable.

pub const VERSION: &str = env!("LIGHTSPEED_BUILD_VERSION");
pub const GIT_SHA: &str = env!("LIGHTSPEED_BUILD_GIT_SHA");
pub const RUST_VERSION: &str = env!("LIGHTSPEED_BUILD_RUST_VERSION");
pub const TARGET: &str = env!("LIGHTSPEED_BUILD_TARGET");
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
