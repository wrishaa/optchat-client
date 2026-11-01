//! OptChat's engine client, shared by the macOS and iOS apps: connections and the app state (no views).
#![recursion_limit = "256"]
pub mod client;
pub mod media;
pub mod mic;
pub mod respell;
pub mod state;
pub mod timer;
pub mod tokens;

/// process start (set by the app first thing), for launch timings
pub static LAUNCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
