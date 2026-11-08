//! `nca` - headless NC preflight checker.
//!
//! Phase 0 stub. `check`, `view`, `simulate`, and `rules` are implemented by the
//! Phase 3 CLI milestone. This binary must never depend on GPUI or `nc-app`.

use nc_analysis::{REPORT_SCHEMA, RULESET_VERSION};

fn main() {
    let mut args = std::env::args();
    let _program = args.next();
    match args.next().as_deref() {
        Some("--version") | Some("version") => {
            println!(
                "nca {} ({REPORT_SCHEMA}, rules {RULESET_VERSION})",
                env!("CARGO_PKG_VERSION")
            );
        }
        _ => {
            eprintln!("nca: headless checker is not implemented yet");
            eprintln!("usage: nca check <file> [--config <toml>] [--format json|markdown|text]");
            std::process::exit(2);
        }
    }
}
