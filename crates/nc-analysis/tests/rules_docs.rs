//! Verifies that the documented rule catalog matches the registry.
//!
//! `docs/RULES.md` is the human contract for rule ids, severity, basis and
//! stock requirement. The table is parsed here and compared with
//! `rules::all()` so a row cannot silently drift.

use std::collections::BTreeMap;
use std::path::Path;

use nc_analysis::rules;
use nc_analysis::{Basis, Severity, StockRequirement};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Documented {
    severity: Severity,
    basis: Basis,
    stock: StockRequirement,
}

fn parse_docs() -> BTreeMap<String, Documented> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/RULES.md");
    let text = std::fs::read_to_string(&path).expect("docs/RULES.md");
    let mut rows = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("| `") else {
            continue;
        };
        let id = match rest.split_once('`') {
            Some((id, _)) => id,
            None => continue,
        };
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() < 6 {
            continue;
        }
        let severity = match cells[1] {
            "Error" => Severity::Error,
            "Warning" => Severity::Warning,
            "Info" => Severity::Info,
            _ => continue,
        };
        let basis = match cells[2] {
            "Exact" => Basis::Exact,
            "Model" => Basis::Model,
            _ => continue,
        };
        let stock = match cells[3] {
            "-" => StockRequirement::None,
            "inferred ok" => StockRequirement::InferredOk,
            "explicit" => StockRequirement::Explicit,
            other => panic!("unknown stock column {other:?} for {id}"),
        };
        let previous = rows.insert(
            id.to_string(),
            Documented {
                severity,
                basis,
                stock,
            },
        );
        assert!(previous.is_none(), "duplicate docs row for {id}");
    }
    rows
}

#[test]
fn rules_docs_match_the_registry() {
    let documented = parse_docs();
    let mut registry = BTreeMap::new();
    for rule in rules::all() {
        let id = rule.id().to_string();
        let previous = registry.insert(
            id.clone(),
            Documented {
                severity: rule.default_severity(),
                basis: rule.basis(),
                stock: rule.requirements().stock,
            },
        );
        assert!(previous.is_none(), "duplicate registry rule {id}");
    }

    let missing: Vec<&String> = registry
        .keys()
        .filter(|id| !documented.contains_key(*id))
        .collect();
    let extra: Vec<&String> = documented
        .keys()
        .filter(|id| !registry.contains_key(*id))
        .collect();
    assert!(missing.is_empty(), "undocumented rules: {missing:?}");
    assert!(
        extra.is_empty(),
        "documented rules not in registry: {extra:?}"
    );

    for (id, expected) in &registry {
        let actual = documented.get(id).expect("documented rule");
        assert_eq!(
            *actual, *expected,
            "docs/RULES.md disagrees with the registry for {id}"
        );
    }
}

#[test]
fn every_documented_rule_has_a_why_it_matters() {
    for rule in rules::all() {
        assert!(
            !rule.why_it_matters().trim().is_empty(),
            "{} has no why-it-matters text",
            rule.id()
        );
    }
}
