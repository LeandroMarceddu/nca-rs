//! Fixture loading and golden-report comparison.
//!
//! Phase 0 stub. The Phase 3 fixtures milestone implements folder discovery,
//! config loading, and reviewed golden reports.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// One fixture directory name requirement.
pub const REQUIRED_FILES: [&str; 4] = ["source.nc", "config.toml", "expected.json", "NOTES.md"];
