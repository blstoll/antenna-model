//! Guards that the served warning-code vocabulary, the OpenAPI spec, and the API
//! documentation all describe the same set of codes (roadmap unit C8 stage 3).
//!
//! The sibling of `error_code_vocabulary.rs`, and deliberately built the same way,
//! because the two vocabularies have the same failure mode: `WarningCode` is
//! compiler-enforced at every emission site, while `openapi.yaml` and
//! `docs/api-documentation.md` are hand-maintained and drift silently.
//!
//! `WarningCode` being a closed enum does half the job already — a producer cannot
//! emit a code that is not in the type. These tests do the other half: they stop a
//! code from being *added* without being documented, which is what C7's freeze will
//! otherwise ratify.

use antenna_model::warnings::WarningCode;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Every code in `WarningCode::ALL` appears in openapi.yaml's `ApiWarning.code`
/// enum, and the enum contains nothing else.
#[test]
fn openapi_warning_enum_matches_the_served_vocabulary() {
    let path = repo_root().join("openapi.yaml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let spec: serde_yaml::Value =
        serde_yaml::from_str(&text).expect("openapi.yaml must be valid YAML");

    let enum_node = spec
        .get("components")
        .and_then(|n| n.get("schemas"))
        .and_then(|n| n.get("ApiWarning"))
        .and_then(|n| n.get("properties"))
        .and_then(|n| n.get("code"))
        .and_then(|n| n.get("enum"))
        .and_then(|n| n.as_sequence())
        .expect("openapi.yaml must define components.schemas.ApiWarning.properties.code.enum");

    let documented: Vec<&str> = enum_node
        .iter()
        .map(|v| {
            v.as_str()
                .expect("every ApiWarning.code enum entry must be a string")
        })
        .collect();

    for code in WarningCode::ALL {
        assert!(
            documented.contains(&code.as_str()),
            "warning code {:?} is served but missing from openapi.yaml's \
             ApiWarning.code enum",
            code.as_str()
        );
    }
    for code in &documented {
        assert!(
            WarningCode::ALL.iter().any(|c| &c.as_str() == code),
            "openapi.yaml documents warning code {code:?}, which the service never emits"
        );
    }
}

/// Every code has a row in the warning-code table in `docs/api-documentation.md`.
///
/// Matches the table-cell form (`` | `code` | ``) rather than a bare substring, so a
/// passing mention in prose does not satisfy the check. That matters more here than
/// for error codes: several warning codes are substrings of nothing, but
/// `extrapolated` appears inside `points_extrapolated`, and `out_of_coverage`
/// appears in the correction-surface prose.
#[test]
fn every_warning_code_is_documented_in_the_api_docs() {
    let path = repo_root().join("docs/api-documentation.md");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));

    for code in WarningCode::ALL {
        let row = format!("| `{}` |", code.as_str());
        assert!(
            text.contains(&row),
            "warning code {:?} has no row in the warning-code table in \
             docs/api-documentation.md (looked for {row:?})",
            code.as_str()
        );
    }
}

/// The published contract carries no bare-string warnings.
///
/// Before C8 stage 3 every `warnings` array held prose. A documented example that
/// still shows `"warnings": ["some sentence"]` would advertise the pre-stage-3
/// shape, and — unlike the request examples — nothing else would catch it in files
/// the C11 deserialization guard does not cover.
#[test]
fn no_bare_string_warnings_remain_in_the_published_contract() {
    let files = [
        "openapi.yaml",
        "docs/api-documentation.md",
        "docs/architecture.md",
        "examples/api_requests.json",
    ];

    for file in files {
        let path = repo_root().join(file);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));

        for (lineno, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            // A warnings array opening with a quote is a string element; the typed
            // form opens with `{` or `[` (empty) or spans lines starting with `{`.
            for marker in ["\"warnings\": [\"", "warnings: [\""] {
                assert!(
                    !trimmed.contains(marker),
                    "{file}:{} shows a bare-string warning; warnings are \
                     {{code, message}} objects since C8 stage 3",
                    lineno + 1
                );
            }
        }
    }
}

/// The vocabulary is non-empty and every code is unique on the wire.
///
/// Cheap backstop for a hand-maintained `ALL`: a duplicated or forgotten entry
/// would quietly weaken both drift tests above (they iterate `ALL`).
#[test]
fn the_vocabulary_is_non_empty_and_unique() {
    assert!(!WarningCode::ALL.is_empty());
    let mut seen = std::collections::HashSet::new();
    for code in WarningCode::ALL {
        assert!(
            seen.insert(code.as_str()),
            "duplicate wire code {:?} in WarningCode::ALL",
            code.as_str()
        );
    }
}
