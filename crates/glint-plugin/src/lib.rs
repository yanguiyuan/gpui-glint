//! Reusable plugin contracts. This crate has no GPUI, HTTP or database dependency.
//! Hosts own installation, grants, persistence and process isolation.
#[cfg(feature = "boa")]
mod crypto;
#[cfg(feature = "boa")]
mod engine;
mod manifest;
mod page;
pub use page::{Node, Page};
/// Editor declarations for the versioned, synchronous JavaScript boundary.
pub const TYPESCRIPT_API: &str = include_str!("glint-plugin.d.ts");
#[cfg(feature = "boa")]
pub use engine::{HostRegistry, Limits, PluginRuntime};
pub use manifest::{Approval, Contribution, Manifest, Origin, Plugin};

/// Stable, credential-safe diagnostic. JavaScript exception messages are intentionally excluded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginError(pub String);
impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for PluginError {}
pub type Result<T> = std::result::Result<T, PluginError>;

#[cfg(all(test, feature = "boa"))]
mod tests;
