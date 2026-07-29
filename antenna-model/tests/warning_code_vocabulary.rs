//! Guards that the served warning-code vocabulary, the OpenAPI spec, and the API
//! documentation all describe the same set of codes (roadmap unit C8 stage 3).
//!
//! The sibling of `error_code_vocabulary.rs`, and deliberately built the same way.
//! `WarningCode` is compiler-enforced at every emission site;
//! `docs/api-documentation.md` is hand-maintained and pinned here, while
//! `openapi.yaml` is generated from the enum (C7) and its check doubles as a
//! utoipa-upgrade canary.
//!
//! `WarningCode` being a closed enum does half the job already — a producer cannot
//! emit a code that is not in the type. These tests do the other half: they stop a
//! code from being *added* without being documented.

use antenna_model::warnings::WarningCode;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Every code in `WarningCode::ALL` appears in openapi.yaml's `WarningCode`
/// enum, the enum contains nothing else, and `ApiWarning.code` references it.
///
/// Since the C7 cutover openapi.yaml is *generated* from the same enum, so this
/// cannot fail through hand-editing drift anymore. It is kept as an upgrade
/// canary: a utoipa version bump that silently changed enum emission (dropped
/// variants, stopped `$ref`-ing, renamed the component) would pass the
/// generate-and-diff test — the file would faithfully match the code — and fail
/// here.
#[test]
fn openapi_warning_enum_matches_the_served_vocabulary() {
    let path = repo_root().join("openapi.yaml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let spec: serde_yaml::Value =
        serde_yaml::from_str(&text).expect("openapi.yaml must be valid YAML");

    let schemas = spec
        .get("components")
        .and_then(|n| n.get("schemas"))
        .expect("openapi.yaml must define components.schemas");

    // The wiring: ApiWarning.code must reference the WarningCode component.
    let code_ref = schemas
        .get("ApiWarning")
        .and_then(|n| n.get("properties"))
        .and_then(|n| n.get("code"))
        .and_then(|n| n.get("$ref"))
        .and_then(|n| n.as_str())
        .expect("ApiWarning.properties.code must $ref a component");
    assert_eq!(code_ref, "#/components/schemas/WarningCode");

    let enum_node = schemas
        .get("WarningCode")
        .and_then(|n| n.get("enum"))
        .and_then(|n| n.as_sequence())
        .expect("openapi.yaml must define components.schemas.WarningCode.enum");

    let documented: Vec<&str> = enum_node
        .iter()
        .map(|v| {
            v.as_str()
                .expect("every WarningCode enum entry must be a string")
        })
        .collect();

    for code in WarningCode::ALL {
        assert!(
            documented.contains(&code.as_str()),
            "warning code {:?} is served but missing from openapi.yaml's \
             WarningCode enum",
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

/// Every `warnings` array in the parseable published contract holds objects, not
/// prose strings.
///
/// Before C8 stage 3 every `warnings` array held prose. A documented example that
/// still shows `"warnings": ["some sentence"]` would advertise the pre-stage-3
/// shape, and — unlike the request examples — nothing else would catch it in files
/// the C11 deserialization guard does not cover.
///
/// The check parses each file and walks it, rather than matching source lines. The
/// first version of this test looked for the one-line forms `"warnings": ["` and
/// `warnings: ["` and was therefore vacuous: `openapi.yaml` writes its examples as
/// YAML block sequences (`warnings:` / `- code: …` on the following lines) and the
/// JSON examples span multiple lines too, so *no* file it scanned could ever match,
/// and reverting an example to bare strings left it green.
#[test]
fn no_bare_string_warnings_remain_in_the_parseable_contract() {
    for file in structured_contract_files() {
        let path = repo_root().join(&file);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        // serde_yaml parses JSON too — JSON is a subset of YAML 1.2.
        let doc: serde_yaml::Value = serde_yaml::from_str(&text)
            .unwrap_or_else(|e| panic!("{file} must parse as YAML/JSON: {e}"));

        assert_no_bare_string_warnings(&doc, &file, "$");
    }
}

/// The same check for the Markdown docs, whose examples live in fenced JSON blocks.
///
/// These cannot be parsed as a whole file, so the scan is line-based — but on the
/// *element* form (an array entry opening with a quote), which is what the examples
/// actually use, rather than the single-line form that never appears.
#[test]
fn no_bare_string_warnings_remain_in_the_markdown_docs() {
    let files = [
        "docs/api-documentation.md",
        "docs/architecture.md",
        "examples/README.md",
        "examples/TESTING.md",
        "examples/QUICKSTART.md",
    ];

    for file in files {
        let path = repo_root().join(file);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));

        let mut in_fence = false;
        let mut in_warnings_array = false;
        // Depth of `{}` nesting inside the array: an element's own fields also open
        // with a quote, so only lines at depth 0 are array elements.
        let mut object_depth = 0i32;

        for (idx, line) in text.lines().enumerate() {
            let lineno = idx + 1;
            let trimmed = line.trim();

            if trimmed.starts_with("```") {
                in_fence = !in_fence;
                in_warnings_array = false;
                continue;
            }
            if !in_fence {
                continue;
            }

            if in_warnings_array {
                if object_depth == 0 {
                    if trimmed.starts_with(']') {
                        in_warnings_array = false;
                        continue;
                    }
                    assert!(
                        !trimmed.starts_with('"'),
                        "{file}:{lineno} shows a bare-string warning ({trimmed}); warnings \
                         are {{code, message}} objects since C8 stage 3"
                    );
                }
                object_depth += trimmed.matches('{').count() as i32;
                object_depth -= trimmed.matches('}').count() as i32;
                continue;
            }

            // `"warnings": [` — possibly with elements or the closing bracket on the
            // same line.
            let Some(rest) = trimmed
                .split_once("\"warnings\"")
                .and_then(|(_, r)| r.trim_start().strip_prefix(':'))
                .map(str::trim_start)
                .and_then(|r| r.strip_prefix('['))
            else {
                continue;
            };
            let rest = rest.trim_start();
            assert!(
                !rest.starts_with('"'),
                "{file}:{lineno} shows a bare-string warning ({trimmed}); warnings are \
                 {{code, message}} objects since C8 stage 3"
            );
            in_warnings_array = !rest.starts_with(']');
            object_depth = 0;
        }
    }
}

/// Files in the published contract that parse as YAML or JSON in full.
///
/// `examples/responses/` is enumerated rather than listed so that a response example
/// added later is covered without anyone remembering to extend this test.
fn structured_contract_files() -> Vec<String> {
    let mut files = vec![
        "openapi.yaml".to_string(),
        "examples/api_requests.json".to_string(),
        "examples/postman_collection.json".to_string(),
    ];

    let responses = repo_root().join("examples/responses");
    let entries = std::fs::read_dir(&responses)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", responses.display()));
    let mut response_files: Vec<String> = entries
        .map(|entry| entry.expect("readable dir entry").path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .map(|p| {
            format!(
                "examples/responses/{}",
                p.file_name().expect("named file").to_string_lossy()
            )
        })
        .collect();
    assert!(
        !response_files.is_empty(),
        "examples/responses/ holds no JSON files; the guard would be vacuous"
    );
    response_files.sort();

    files.extend(response_files);
    files
}

/// Recursively assert that no mapping key named `warnings` holds a string element.
fn assert_no_bare_string_warnings(node: &serde_yaml::Value, file: &str, path: &str) {
    match node {
        serde_yaml::Value::Mapping(map) => {
            for (key, value) in map {
                let key_str = key.as_str().unwrap_or("<non-string key>");
                let child = format!("{path}.{key_str}");

                if key_str == "warnings" {
                    if let Some(items) = value.as_sequence() {
                        for (i, item) in items.iter().enumerate() {
                            assert!(
                                !item.is_string(),
                                "{file}: {child}[{i}] is a bare string ({:?}); warnings are \
                                 {{code, message}} objects since C8 stage 3",
                                item.as_str().unwrap_or_default()
                            );
                        }
                    }
                }

                assert_no_bare_string_warnings(value, file, &child);
            }
        }
        serde_yaml::Value::Sequence(items) => {
            for (i, item) in items.iter().enumerate() {
                assert_no_bare_string_warnings(item, file, &format!("{path}[{i}]"));
            }
        }
        _ => {}
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
