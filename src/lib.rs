//! shephrd as a library. `main.rs` is a thin CLI on top of this, and the
//! integration tests in `tests/` use it directly.

pub mod claude_request;
pub mod client;
pub mod config;
pub mod judges;
pub mod prompt;
pub mod server;
pub mod settings;
pub mod transcript;
