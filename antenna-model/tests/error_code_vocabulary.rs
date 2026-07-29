//! Guards that the served error-code vocabulary, the OpenAPI spec, and the API
//! documentation all describe the same set of codes (roadmap unit C3).
//!
//! `api::schemas::ErrorCode` is the source of truth and is compiler-enforced at
//! every emission site. `docs/api-documentation.md` is hand-maintained, so it is
//! pinned here; `openapi.yaml` is generated from the enum (C7), and its check
//! doubles as a utoipa-upgrade canary.
//!
//! This is deliberately narrow: it checks the *vocabulary*, not the status codes
//! each one is served with. Roadmap unit C2 owns the statuses; C7's
//! generate-and-diff test (`openapi_spec.rs`) and route cross-check
//! (`openapi_routes_match.rs`) own the rest of the spec.

use antenna_model::api::schemas::ErrorCode;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Every code in `ErrorCode::ALL` appears in openapi.yaml's `ErrorCode` component
/// enum, the enum contains nothing else, and both `ErrorResponse.error` and
/// `GainError.code` reference that one component.
///
/// Before the C7 cutover the two fields carried duplicated inline enum copies and
/// both had to be checked; generation collapsed them into one `$ref`'d component.
/// Since openapi.yaml is now *generated* from the same enum, this cannot fail
/// through hand-editing drift anymore — it is kept as an upgrade canary: a utoipa
/// version bump that silently changed enum emission (dropped variants, stopped
/// `$ref`-ing, renamed the component) would pass the generate-and-diff test and
/// fail here.
#[test]
fn openapi_error_enum_matches_the_served_vocabulary() {
    let path = repo_root().join("openapi.yaml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let spec: serde_yaml::Value =
        serde_yaml::from_str(&text).expect("openapi.yaml must be valid YAML");

    let schemas = spec
        .get("components")
        .and_then(|n| n.get("schemas"))
        .expect("openapi.yaml must define components.schemas");

    // The wiring: both error-code fields must reference the ErrorCode component.
    for (schema, property) in [("ErrorResponse", "error"), ("GainError", "code")] {
        let code_ref = schemas
            .get(schema)
            .and_then(|n| n.get("properties"))
            .and_then(|n| n.get(property))
            .and_then(|n| n.get("$ref"))
            .and_then(|n| n.as_str())
            .unwrap_or_else(|| panic!("{schema}.properties.{property} must $ref a component"));
        assert_eq!(code_ref, "#/components/schemas/ErrorCode");
    }

    let documented: Vec<&str> = schemas
        .get("ErrorCode")
        .and_then(|n| n.get("enum"))
        .and_then(|n| n.as_sequence())
        .expect("openapi.yaml must define components.schemas.ErrorCode.enum")
        .iter()
        .map(|v| {
            v.as_str()
                .expect("every ErrorCode enum entry must be a string")
        })
        .collect();

    for code in ErrorCode::ALL {
        assert!(
            documented.contains(&code.as_str()),
            "error code {code:?} is served but missing from openapi.yaml's ErrorCode enum"
        );
    }
    for code in &documented {
        assert!(
            ErrorCode::ALL.iter().any(|c| c.as_str() == *code),
            "openapi.yaml's ErrorCode enum documents error code {code:?}, \
             which the service never emits"
        );
    }
}

/// Every code has a row in the error-code table in `docs/api-documentation.md`.
///
/// Matches on the table-cell form (`` | `code` | ``) rather than a bare substring, so
/// a passing mention in prose does not satisfy the check — and so that
/// `validation_error` cannot stand in for a missing `invalid_request_body` row by
/// virtue of one code being a substring of some other line.
#[test]
fn every_error_code_is_documented_in_the_api_docs() {
    let path = repo_root().join("docs/api-documentation.md");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));

    for code in ErrorCode::ALL {
        let row = format!("| `{code}` |");
        assert!(
            text.contains(&row),
            "error code {code:?} has no row in the error-code table in \
             docs/api-documentation.md (looked for {row:?})"
        );
    }
}

/// No `PascalCase` error code survives in the spec or the published examples.
///
/// Before C3, `openapi.yaml`, `docs/architecture.md`, and `examples/api_requests.json`
/// all advertised codes like `"AntennaNotFound"` that no emission site ever produced —
/// they came from a set of unused `ErrorResponse` convenience constructors. The
/// constructors are gone; this keeps the documentation from regrowing them.
#[test]
fn no_pascal_case_error_codes_remain_in_the_published_contract() {
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
            // Only the wire field matters: `error: "Foo"` (YAML) or `"error": "Foo"`
            // (JSON/Markdown snippets). Rust type names in prose are fine.
            for marker in ["error: \"", "\"error\": \""] {
                let Some(rest) = line.split_once(marker).map(|(_, r)| r) else {
                    continue;
                };
                let Some(value) = rest.split('"').next() else {
                    continue;
                };
                assert!(
                    !value.starts_with(char::is_uppercase),
                    "{file}:{} advertises PascalCase error code {value:?}; the served \
                     vocabulary is snake_case (see api/schemas.rs, enum ErrorCode)",
                    lineno + 1
                );
            }
        }
    }
}
