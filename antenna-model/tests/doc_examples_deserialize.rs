//! Guards that every request/response example embedded in the prose docs actually
//! deserializes into the schema it claims to show.
//!
//! G3 pinned `examples/requests/*.json`, but the same examples are duplicated inside
//! fenced code blocks in `docs/`, and nothing checked those. They rotted: the cURL and
//! JavaScript examples in `docs/api-documentation.md` carried a `{"w": …}` object for
//! `vehicle_attitude` long after the schema moved to a `[w, x, y, z]` array — the exact
//! break G3 fixed in the JSON files, surviving untouched in the prose beside them.
//!
//! # How to add a documented example
//!
//! Put a marker comment immediately before the fenced block (HTML comments do not
//! render in Markdown, so readers never see it):
//!
//! ```markdown
//! <!-- api-example: GainRequest -->
//! ```json
//! { "antenna_id": "...", ... }
//! ```
//! ```
//!
//! The block's language decides how the payload is extracted:
//!
//! | fence | payload |
//! |---|---|
//! | `json` | the whole block |
//! | `bash` | the argument of `-d '…'` (a cURL body) |
//! | `javascript` | the argument of `JSON.stringify(…)` |
//!
//! `javascript` payloads must therefore be written with quoted keys — which is still
//! valid JavaScript, so the example reads normally and is machine-checkable.
//!
//! An unmarked block that looks like an API example is a hard error rather than a
//! silent gap: see `every_api_example_block_is_marked`.

use antenna_model::api::schemas::{
    BatchGainRequest, ErrorResponse, GainRequest, GainResponse, H3LinkBudgetRequest,
    H3LinkBudgetResponse, HeatmapRequest,
};
use std::path::{Path, PathBuf};

const MARKER: &str = "<!-- api-example:";

/// A marked example: which schema it claims, and the payload to check against it.
struct DocExample {
    file: String,
    line: usize,
    schema: String,
    payload: String,
}

/// Docs whose examples are part of the published API contract, and are therefore held
/// to the schemas.
///
/// Deliberately *not* all of `docs/`. The design and workflow documents
/// (`architecture.md`, `partial-calibration-design.md`, …) also contain JSON blocks, but
/// they are illustrative and in places knowingly aspirational; auditing them is roadmap
/// unit D5's job, not this guard's. Adding a file here is one line — do that as D5 makes
/// each one true, so the guard ratchets forward instead of blocking on a sweep.
const CONTRACT_DOCS: [&str; 1] = ["api-documentation.md"];

fn docs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs")
}

fn markdown_files() -> Vec<PathBuf> {
    CONTRACT_DOCS
        .iter()
        .map(|name| {
            let path = docs_dir().join(name);
            assert!(path.is_file(), "missing contract doc: {}", path.display());
            path
        })
        .collect()
}

/// Pull the JSON payload out of one fenced block, given its language tag.
///
/// Returns `None` for a language this guard does not know how to read, so an
/// unsupported fence fails loudly at the call site rather than being skipped.
fn extract_payload(lang: &str, body: &str) -> Option<String> {
    match lang {
        "json" => Some(body.to_string()),
        // cURL: -d '<json>'. The body is the last single-quoted argument.
        "bash" => {
            let after = body.split("-d ").nth(1)?;
            let rest = after.strip_prefix('\'')?;
            let end = rest.rfind('\'')?;
            Some(rest[..end].to_string())
        }
        // fetch(): JSON.stringify(<object literal>) — balance parens from the call.
        "javascript" => {
            let start = body.find("JSON.stringify(")? + "JSON.stringify(".len();
            let mut depth = 1usize;
            for (i, c) in body[start..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(body[start..start + i].to_string());
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        _ => None,
    }
}

/// Collect every marked example across `docs/*.md`.
fn collect_examples() -> Vec<DocExample> {
    let mut found = Vec::new();

    for path in markdown_files() {
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        let lines: Vec<&str> = text.lines().collect();

        for (idx, line) in lines.iter().enumerate() {
            let Some(rest) = line.trim().strip_prefix(MARKER) else {
                continue;
            };
            let schema = rest
                .trim()
                .strip_suffix("-->")
                .unwrap_or_else(|| panic!("{name}:{}: malformed marker: {line}", idx + 1))
                .trim()
                .to_string();

            // The fence must open on the next non-blank line.
            let mut cursor = idx + 1;
            while cursor < lines.len() && lines[cursor].trim().is_empty() {
                cursor += 1;
            }
            let open = lines.get(cursor).unwrap_or_else(|| {
                panic!("{name}:{}: marker is not followed by a code block", idx + 1)
            });
            let lang = open.trim().strip_prefix("```").unwrap_or_else(|| {
                panic!(
                    "{name}:{}: marker must be followed by a fenced block, found: {open}",
                    idx + 1
                )
            });

            let body_start = cursor + 1;
            let body_end = (body_start..lines.len())
                .find(|&i| lines[i].trim() == "```")
                .unwrap_or_else(|| panic!("{name}:{}: unterminated code block", cursor + 1));
            let body = lines[body_start..body_end].join("\n");

            let payload = extract_payload(lang.trim(), &body).unwrap_or_else(|| {
                panic!(
                    "{name}:{}: cannot extract a JSON payload from a `{lang}` block \
                     (supported: json, bash with -d '…', javascript with JSON.stringify(…))",
                    cursor + 1
                )
            });

            found.push(DocExample {
                file: name.clone(),
                line: cursor + 1,
                schema,
                payload,
            });
        }
    }

    found
}

fn check<T: serde::de::DeserializeOwned>(example: &DocExample) {
    if let Err(e) = serde_json::from_str::<T>(&example.payload) {
        panic!(
            "{}:{}: documented example does not deserialize into {}: {e}\n---\n{}\n---",
            example.file,
            example.line,
            example.schema,
            example.payload.trim()
        );
    }
}

#[test]
fn every_documented_example_deserializes() {
    let examples = collect_examples();

    for example in &examples {
        match example.schema.as_str() {
            "GainRequest" => check::<GainRequest>(example),
            "BatchGainRequest" => check::<BatchGainRequest>(example),
            "HeatmapRequest" => check::<HeatmapRequest>(example),
            "H3LinkBudgetRequest" => check::<H3LinkBudgetRequest>(example),
            "GainResponse" => check::<GainResponse>(example),
            "H3LinkBudgetResponse" => check::<H3LinkBudgetResponse>(example),
            "ErrorResponse" => check::<ErrorResponse>(example),
            other => panic!(
                "{}:{}: unknown schema `{other}` — add it to \
                 every_documented_example_deserializes",
                example.file, example.line
            ),
        }
    }

    assert!(
        examples.len() >= 8,
        "expected the documented examples to still be marked, found only {}",
        examples.len()
    );
}

/// A block that looks like an API example but carries no marker is a coverage hole —
/// exactly how the `vehicle_attitude` break survived. Fail rather than skip it.
#[test]
fn every_api_example_block_is_marked() {
    // Fields distinctive enough that a block containing one is an API payload, not
    // prose or an unrelated config snippet.
    const TELLS: [&str; 3] = ["\"antenna_id\"", "antenna_id:", "\"error\":"];

    let mut unmarked = Vec::new();

    for path in markdown_files() {
        let text = std::fs::read_to_string(&path).expect("readable markdown");
        let name = path.file_name().unwrap().to_str().unwrap();
        let lines: Vec<&str> = text.lines().collect();

        let mut i = 0usize;
        while i < lines.len() {
            let Some(lang) = lines[i].trim().strip_prefix("```") else {
                i += 1;
                continue;
            };
            let body_start = i + 1;
            let Some(body_end) = (body_start..lines.len()).find(|&j| lines[j].trim() == "```")
            else {
                break;
            };
            let body = lines[body_start..body_end].join("\n");

            let looks_like_api = TELLS.iter().any(|t| body.contains(t));
            let marked = lines[..i]
                .iter()
                .rev()
                .take_while(|l| l.trim().is_empty() || l.trim().starts_with(MARKER))
                .any(|l| l.trim().starts_with(MARKER));

            if looks_like_api && !marked && extract_payload(lang.trim(), &body).is_some() {
                unmarked.push(format!("{name}:{}", i + 1));
            }

            i = body_end + 1;
        }
    }

    assert!(
        unmarked.is_empty(),
        "these code blocks look like API examples but carry no `{MARKER} Type -->` \
         marker, so nothing checks them against the schema: {unmarked:?}"
    );
}
