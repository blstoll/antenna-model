//! Validates every JSON example this repo publishes — the `examples/` tree
//! **and `openapi.yaml`'s own inline examples** — against the published
//! `openapi.yaml` component schemas. Roadmap unit **C15 option 3**, which is
//! C7's stretch goal ("validate G3's example files against the openapi
//! component schemas") promoted to required.
//!
//! # Why this exists when three deserialize guards already do
//!
//! `example_requests_deserialize.rs` (G3), `example_responses_deserialize.rs`
//! (C8 stage 1) and `example_api_requests_deserialize.rs` (C15 option 1) each
//! check an example against a **Rust type**. C15's inventory of uncovered
//! client-visible surfaces recorded that *nothing* checks `openapi.yaml`
//! itself, and named option 3 as "the only mechanism that would cover
//! `openapi.yaml`". This is that mechanism, and it is not redundant with the
//! three:
//!
//! * The spec is generated from the types (C7), so type and spec agree **by
//!   construction only where utoipa saw the truth**. Where it did not — a
//!   `#[schema(value_type = …)]` override that lies, a field utoipa cannot
//!   see, a custom serializer whose wire shape the derive cannot know (the C7
//!   `nan_as_null` hazard) — the type says one thing and the published
//!   contract says another. A serde round-trip cannot see that gap; only
//!   checking an example against the *spec* can.
//! * It checks assertions serde has no concept of: `enum` membership,
//!   `minimum`, tuple arity (`prefixItems` + `items: false`), and whether
//!   `null` is an allowed type at a given position.
//! * It covers `examples/postman_collection.json`, which no guard read before,
//!   and the spec's own inline `content.application/json.examples` bodies —
//!   the ones Swagger UI and Redoc actually render, i.e. the closest thing
//!   this repo has to what a client is shown.
//!
//! # What is checked, and against what
//!
//! Every example is validated against the schema the spec attaches to the
//! **endpoint it claims to belong to** — a `(method, path)` request body, or a
//! `(method, path, status)` response — rather than against a hand-picked
//! component name. An example that is valid `GainRequest`-shaped JSON but is
//! filed under the batch endpoint is a defect this framing catches and a
//! name→type map does not.
//!
//! Five sources feed it, each with its own count assertion so a surface that
//! stops being discovered fails naming itself:
//!
//! | source | bodies |
//! |---|---|
//! | `examples/requests/*.json` | 10 |
//! | `examples/responses/*.json` | 8 |
//! | `examples/api_requests.json` | 16 |
//! | `examples/postman_collection.json` | 4 |
//! | `openapi.yaml` inline examples | 38 |
//!
//! The postman collection contributes 4 **bodies** but 11 **requests**: its
//! seven GETs have nothing to validate, so for them the assertion is that
//! their URL still resolves to a documented `(method, path)` template. A
//! C8-stage-4-style endpoint removal would otherwise leave one of them
//! shipping a 404 with every guard green.
//!
//! The spec is read from the **committed `openapi.yaml`**, not from
//! `ApiDoc::openapi()`. That file is the artifact clients consume;
//! `tests/openapi_spec.rs` already pins it byte-for-byte to the generated
//! document, so reading the file adds coverage of the committed artifact
//! without weakening anything.
//!
//! # Three properties that keep this guard from rotting
//!
//! 1. **An unhandled schema keyword is a hard failure**, not a silent skip
//!    (`the_spec_uses_no_schema_keyword_this_validator_understands_nothing_of`).
//!    A JSON Schema validator that ignores what it does not know gets weaker
//!    every time the spec grows a construct — it keeps passing while checking
//!    less, which is precisely the rot P13 records. If utoipa starts emitting
//!    `maximum`, `pattern` or `allOf`, this test fails until the validator
//!    learns the keyword. `KNOWN_KEYWORDS` is split into `ENFORCED_KEYWORDS`
//!    and `ANNOTATION_KEYWORDS` so a new assertive keyword cannot be waved
//!    through by adding it to a list of things that mean nothing.
//! 2. **A keyword that is present but malformed panics**, rather than quietly
//!    asserting nothing. `minimum`, `minItems` and `maxItems` used to reach
//!    for `as_f64()`/`as_u64()` and skip on `None`, so a bound arriving as a
//!    string or a fraction would have removed the assertion with the suite
//!    still green — the same silent-weakening class as (1), one level down.
//!    Every keyword position now fails loudly on a shape it cannot read.
//! 3. **Every negative control is paired with a positive one.** The validator
//!    accepts all 76 example bodies as they stand, so "the suite is green" is
//!    by itself no evidence that it can reject anything. Twelve controls
//!    mutate a known-good example one way each (drop a required field, wrong
//!    type, undeclared key, a nested `_` key, bad enum value, below
//!    `minimum`, `null` where the type forbids it, wrong tuple arity, no
//!    matching `oneOf` branch, an unimplemented keyword, a malformed
//!    `minimum`/`minItems`, an undocumented postman URL) and each must be
//!    caught.
//!
//! # Known deliberate exemption: `_`-prefixed annotation keys
//!
//! Five `examples/requests/geo_*.json` files carry top-level `_comment`,
//! `_description`, `_geodetic_satellite`, `_geodetic_ground` and `_notes`
//! keys. JSON has no comments, and this is the file format's substitute:
//! serde ignores them, no client reads them, and they carry no contract. A
//! `_`-prefixed key **at the root of an example** is therefore exempt from the
//! undeclared-property check. Nothing else is: the exemption does not reach
//! nested objects (`a_nested_underscore_key_is_not_exempt`) and does not
//! extend past the prefix
//! (`an_underscore_prefixed_annotation_key_is_exempt_but_nothing_else_is`).
//!
//! # A finding this guard surfaced, filed rather than fixed
//!
//! utoipa emits a Rust `(f64, f64)` as `prefixItems` + `items: false` and **no
//! `minItems`**, so every published `(min, max)` pair — `ValidityRangesInfo`'s
//! three ranges, `CoverageInfo`'s three, `FeedInfo.frequency_range_mhz` — is a
//! schema that accepts `[]` and `[0.0]` as readily as `[0.0, 360.0]`, while
//! serde requires exactly two elements. (A Rust *array*, `[f64; 4]` for
//! `vehicle_attitude`, does get `minItems`/`maxItems`; the gap is tuples
//! specifically.) The over-long case IS caught, by `items: false` — which is
//! what `a_tuple_of_the_wrong_arity_is_caught` pins. Tightening the short case
//! means changing what the frozen spec says, which is a contract change and
//! not this unit's charter; recorded in C15's roadmap section.
//!
//! # What this unit does NOT cover, and why
//!
//! `examples/python_examples.py` builds request bodies as Python dict literals
//! and `examples/QUICKSTART.md` / `TESTING.md` / `README*.md` embed prose
//! examples; extracting either would mean a fragile parser for a non-JSON host
//! language, which is a different unit's work. `examples/curl-examples.sh` was
//! checked and needs nothing — it carries no inline JSON, only
//! `-d @requests/*.json` references to files this guard already validates.
//! These remain open rows of C15's inventory.

use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Keyword vocabulary
// ---------------------------------------------------------------------------

/// Keywords this validator **asserts on**. Adding one here is a promise that
/// `Validator::check` implements it.
const ENFORCED_KEYWORDS: &[&str] = &[
    "$ref",
    "type",
    "enum",
    "required",
    "properties",
    "items",
    "prefixItems",
    "minItems",
    "maxItems",
    "minimum",
    "oneOf",
];

/// Keywords that carry documentation only and are intentionally not asserted
/// on. `format` is an annotation in JSON Schema 2020-12 (`double`, `int64`,
/// `uuid` … constrain nothing on their own), and the underlying `type` is
/// already enforced.
const ANNOTATION_KEYWORDS: &[&str] = &["description", "format"];

fn keyword_is_known(k: &str) -> bool {
    ENFORCED_KEYWORDS.contains(&k) || ANNOTATION_KEYWORDS.contains(&k)
}

// ---------------------------------------------------------------------------
// Spec loading
// ---------------------------------------------------------------------------

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("antenna-model crate lives one level under the repo root")
        .to_path_buf()
}

fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()))
}

/// The committed `openapi.yaml`, as a `serde_json::Value` tree.
struct Spec {
    root: Value,
}

impl Spec {
    fn load() -> Self {
        let path = repo_root().join("openapi.yaml");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let root: Value = serde_yaml::from_str(&text)
            .unwrap_or_else(|e| panic!("{} is not parseable as YAML: {e}", path.display()));
        Self { root }
    }

    fn operation(&self, method: &str, path: &str) -> &Value {
        self.root
            .get("paths")
            .and_then(|p| p.get(path))
            .and_then(|p| p.get(method))
            .unwrap_or_else(|| panic!("openapi.yaml documents no {} {path}", method.to_uppercase()))
    }

    /// Schema of the `application/json` request body of `method path`.
    fn request_schema(&self, method: &str, path: &str) -> &Value {
        self.operation(method, path)
            .get("requestBody")
            .and_then(|b| b.pointer("/content/application~1json/schema"))
            .unwrap_or_else(|| {
                panic!(
                    "{} {path} declares no application/json request body",
                    method.to_uppercase()
                )
            })
    }

    /// Schema of the `application/json` body of the `status` response of
    /// `method path`.
    fn response_schema(&self, method: &str, path: &str, status: &str) -> &Value {
        self.operation(method, path)
            .pointer(&format!(
                "/responses/{status}/content/application~1json/schema"
            ))
            .unwrap_or_else(|| {
                panic!(
                    "{} {path} declares no application/json {status} response",
                    method.to_uppercase()
                )
            })
    }

    fn components(&self) -> &Map<String, Value> {
        self.root
            .pointer("/components/schemas")
            .and_then(|v| v.as_object())
            .expect("openapi.yaml has components.schemas")
    }

    fn component(&self, name: &str) -> &Value {
        self.components()
            .get(name)
            .unwrap_or_else(|| panic!("openapi.yaml has no component schema {name}"))
    }
}

// ---------------------------------------------------------------------------
// The validator
// ---------------------------------------------------------------------------

/// Result of validating one instance: the failures found, and the set of
/// component schemas the instance actually reached.
#[derive(Default)]
struct Report {
    errors: Vec<String>,
    exercised: BTreeSet<String>,
}

impl Report {
    fn ok(&self) -> bool {
        self.errors.is_empty()
    }

    fn absorb(&mut self, other: Report) {
        self.errors.extend(other.errors);
        self.exercised.extend(other.exercised);
    }
}

struct Validator<'a> {
    spec: &'a Spec,
}

impl<'a> Validator<'a> {
    fn new(spec: &'a Spec) -> Self {
        Self { spec }
    }

    fn validate(&self, schema: &Value, instance: &Value) -> Report {
        let mut report = Report::default();
        self.check(schema, instance, "", &mut report);
        report
    }

    fn check(&self, schema: &Value, instance: &Value, at: &str, report: &mut Report) {
        // Boolean schemas: `items: false` (the tail of a `prefixItems` tuple)
        // is the only one this spec emits, but both are trivial to honour.
        match schema {
            Value::Bool(true) => return,
            Value::Bool(false) => {
                report.errors.push(format!(
                    "{}: schema is `false` — nothing is valid here",
                    loc(at)
                ));
                return;
            }
            _ => {}
        }
        let Some(obj) = schema.as_object() else {
            report.errors.push(format!(
                "{}: schema is neither an object nor a boolean ({schema})",
                loc(at)
            ));
            return;
        };

        for key in obj.keys() {
            if !keyword_is_known(key) {
                report.errors.push(format!(
                    "{}: schema uses keyword `{key}`, which this validator does not implement \
                     — teach it the keyword rather than ignoring it (see the module doc)",
                    loc(at)
                ));
            }
        }

        // `$ref`. In 2020-12 siblings of `$ref` apply alongside the target;
        // this spec only ever writes `description` beside one, and silently
        // merging anything else would be inventing semantics.
        if let Some(reference) = obj.get("$ref") {
            let unhandled: Vec<&str> = obj
                .keys()
                .map(String::as_str)
                .filter(|k| *k != "$ref" && !ANNOTATION_KEYWORDS.contains(k))
                .collect();
            if !unhandled.is_empty() {
                report.errors.push(format!(
                    "{}: `$ref` carries sibling keywords {unhandled:?} that this validator does \
                     not combine with the referenced schema",
                    loc(at)
                ));
                return;
            }
            let name = reference
                .as_str()
                .and_then(|r| r.strip_prefix("#/components/schemas/"))
                .unwrap_or_else(|| panic!("{}: unsupported $ref target {reference}", loc(at)));
            report.exercised.insert(name.to_string());
            self.check(self.spec.component(name), instance, at, report);
            return;
        }

        if let Some(branches) = obj.get("oneOf") {
            // Every `oneOf` this spec emits is the whole schema. Combining it
            // with sibling assertions is well-defined in JSON Schema but not
            // implemented here, and quietly dropping them would be the silent
            // weakening this guard exists to prevent.
            let unhandled: Vec<&str> = obj
                .keys()
                .map(String::as_str)
                .filter(|k| *k != "oneOf" && !ANNOTATION_KEYWORDS.contains(k))
                .collect();
            if !unhandled.is_empty() {
                report.errors.push(format!(
                    "{}: `oneOf` carries sibling keywords {unhandled:?} that this validator \
                     does not combine with the branches",
                    loc(at)
                ));
                return;
            }
            self.check_one_of(branches, instance, at, report);
            return;
        }

        if let Some(t) = obj.get("type") {
            if !self.type_matches(t, instance, at, report) {
                // Every remaining assertion presumes the type; reporting them
                // too would bury the one error that matters.
                return;
            }
        }

        if let Some(values) = obj.get("enum") {
            let allowed = values
                .as_array()
                .unwrap_or_else(|| panic!("{}: `enum` is not an array", loc(at)));
            if !allowed.contains(instance) {
                report.errors.push(format!(
                    "{}: {instance} is not one of the {} values the spec allows ({})",
                    loc(at),
                    allowed.len(),
                    allowed
                        .iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }

        if let Some(raw) = obj.get("minimum") {
            let min = raw
                .as_f64()
                .unwrap_or_else(|| panic!("{}: `minimum` is not a number ({raw})", loc(at)));
            if let Some(n) = instance.as_f64() {
                if n < min {
                    report.errors.push(format!(
                        "{}: {n} is below the spec's minimum {min}",
                        loc(at)
                    ));
                }
            }
        }

        if let Some(map) = instance.as_object() {
            self.check_object(obj, map, at, report);
        }
        if let Some(items) = instance.as_array() {
            self.check_array(obj, items, at, report);
        }
    }

    fn check_one_of(&self, branches: &Value, instance: &Value, at: &str, report: &mut Report) {
        let branches = branches
            .as_array()
            .unwrap_or_else(|| panic!("{}: `oneOf` is not an array", loc(at)));
        let mut matched = Vec::new();
        let mut failures = Vec::new();
        for (i, branch) in branches.iter().enumerate() {
            let mut sub = Report::default();
            self.check(branch, instance, at, &mut sub);
            if sub.ok() {
                matched.push(sub);
            } else {
                failures.push(format!("  branch {i}: {}", sub.errors.join("; ")));
            }
        }
        match matched.len() {
            1 => report.absorb(matched.pop().expect("length checked")),
            0 => report.errors.push(format!(
                "{}: matches none of the {} `oneOf` branches:\n{}",
                loc(at),
                branches.len(),
                failures.join("\n")
            )),
            n => report.errors.push(format!(
                "{}: matches {n} `oneOf` branches, which must be exactly 1",
                loc(at)
            )),
        }
    }

    fn check_object(
        &self,
        schema: &Map<String, Value>,
        instance: &Map<String, Value>,
        at: &str,
        report: &mut Report,
    ) {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for name in required {
                let name = name
                    .as_str()
                    .unwrap_or_else(|| panic!("{}: `required` holds a non-string entry", loc(at)));
                if !instance.contains_key(name) {
                    report.errors.push(format!(
                        "{}: the spec requires `{name}`, which the example does not carry",
                        loc(at)
                    ));
                }
            }
        }

        let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
            // No declared properties: the spec constrains nothing about this
            // object's members, so neither does this check.
            return;
        };
        for (key, value) in instance {
            match properties.get(key) {
                Some(sub) => self.check(sub, value, &child(at, key), report),
                // `_`-prefixed keys are the examples' comment convention, and
                // the convention is a **top-level** one — every annotation key
                // in the tree sits at the root of its file. Exempting the
                // prefix at any depth would quietly widen the hole into the
                // undeclared-property check; see the module doc.
                None if at.is_empty() && key.starts_with('_') => {}
                None => report.errors.push(format!(
                    "{}: the example carries `{key}`, which the published schema does not \
                     declare (a field the spec omits, or a stale key)",
                    loc(at)
                )),
            }
        }
    }

    fn check_array(
        &self,
        schema: &Map<String, Value>,
        instance: &[Value],
        at: &str,
        report: &mut Report,
    ) {
        if let Some(raw) = schema.get("minItems") {
            let min = raw.as_u64().unwrap_or_else(|| {
                panic!(
                    "{}: `minItems` is not a non-negative integer ({raw})",
                    loc(at)
                )
            });
            if (instance.len() as u64) < min {
                report.errors.push(format!(
                    "{}: has {} items, fewer than the spec's minItems {min}",
                    loc(at),
                    instance.len()
                ));
            }
        }
        if let Some(raw) = schema.get("maxItems") {
            let max = raw.as_u64().unwrap_or_else(|| {
                panic!(
                    "{}: `maxItems` is not a non-negative integer ({raw})",
                    loc(at)
                )
            });
            if (instance.len() as u64) > max {
                report.errors.push(format!(
                    "{}: has {} items, more than the spec's maxItems {max}",
                    loc(at),
                    instance.len()
                ));
            }
        }

        let prefix = schema
            .get("prefixItems")
            .map(|p| {
                p.as_array()
                    .unwrap_or_else(|| panic!("{}: `prefixItems` is not an array", loc(at)))
                    .as_slice()
            })
            .unwrap_or(&[]);
        for (i, (sub, value)) in prefix.iter().zip(instance).enumerate() {
            self.check(sub, value, &index(at, i), report);
        }
        // Whatever `prefixItems` did not cover falls to `items` — which for
        // the spec's `(min, max)` tuples is `false`, i.e. a length assertion.
        if let Some(items) = schema.get("items") {
            for (i, value) in instance.iter().enumerate().skip(prefix.len()) {
                self.check(items, value, &index(at, i), report);
            }
        }
    }

    fn type_matches(
        &self,
        declared: &Value,
        instance: &Value,
        at: &str,
        report: &mut Report,
    ) -> bool {
        let names: Vec<&str> = match declared {
            Value::String(s) => vec![s.as_str()],
            Value::Array(a) => a
                .iter()
                .map(|v| {
                    v.as_str()
                        .unwrap_or_else(|| panic!("{}: `type` array holds a non-string", loc(at)))
                })
                .collect(),
            other => panic!(
                "{}: `type` is neither a string nor an array ({other})",
                loc(at)
            ),
        };
        let matched = names.iter().any(|n| match *n {
            "object" => instance.is_object(),
            "array" => instance.is_array(),
            "string" => instance.is_string(),
            "boolean" => instance.is_boolean(),
            "number" => instance.is_number(),
            // JSON Schema counts a float with zero fractional part as an
            // integer. Whether the Rust type can actually hold it is the
            // deserialize guards' question, not the spec's.
            "integer" => {
                instance.is_i64()
                    || instance.is_u64()
                    || instance.as_f64().is_some_and(|f| f.fract() == 0.0)
            }
            "null" => instance.is_null(),
            other => panic!("{}: unknown JSON Schema type `{other}`", loc(at)),
        });
        if !matched {
            report.errors.push(format!(
                "{}: the spec declares type {}, the example carries {}",
                loc(at),
                names.join(" | "),
                describe(instance)
            ));
        }
        matched
    }
}

fn loc(at: &str) -> String {
    if at.is_empty() {
        "<root>".to_string()
    } else {
        at.to_string()
    }
}

fn child(at: &str, key: &str) -> String {
    if at.is_empty() {
        key.to_string()
    } else {
        format!("{at}.{key}")
    }
}

fn index(at: &str, i: usize) -> String {
    format!("{}[{i}]", loc(at))
}

fn describe(v: &Value) -> String {
    let kind = match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    };
    let rendered = v.to_string();
    // Slice on a char boundary: example strings carry `°` and other non-ASCII.
    match rendered.char_indices().nth(60) {
        Some((cut, _)) => format!("{kind} ({}…)", &rendered[..cut]),
        None => format!("{kind} ({rendered})"),
    }
}

// ---------------------------------------------------------------------------
// The example inventory
// ---------------------------------------------------------------------------

/// One example body and the endpoint whose schema it must satisfy.
struct Case {
    /// Human-readable origin, used only in failure messages.
    origin: String,
    method: String,
    path: String,
    /// `None` for a request body, `Some(status)` for a response body.
    status: Option<String>,
    body: Value,
}

impl Case {
    fn request(origin: String, method: &str, path: &str, body: Value) -> Self {
        Self {
            origin,
            method: method.to_string(),
            path: path.to_string(),
            status: None,
            body,
        }
    }

    fn response(origin: String, method: &str, path: &str, status: &str, body: Value) -> Self {
        Self {
            origin,
            method: method.to_string(),
            path: path.to_string(),
            status: Some(status.to_string()),
            body,
        }
    }
}

/// `examples/requests/*.json` → the endpoint that accepts them.
fn request_file_endpoint(file: &str) -> (&'static str, &'static str) {
    match file {
        "batch_request.json" => ("post", "/api/v1/gain/batch"),
        "heatmap_request.json" => ("post", "/api/v1/heatmap"),
        "h3_link_budget_request.json" => ("post", "/api/v1/h3-heatmap"),
        // Every single-gain example, including the geo_*.json fixtures — the
        // same partition `example_requests_deserialize.rs` uses.
        f if f.starts_with("gain_request") || f.starts_with("geo_") => ("post", "/api/v1/gain"),
        other => panic!(
            "no endpoint mapping for examples/requests/{other} — add it to \
             request_file_endpoint"
        ),
    }
}

/// `examples/responses/*.json` → the endpoint and status that produce them.
fn response_file_endpoint(file: &str) -> (&'static str, &'static str, &'static str) {
    match file {
        "gain_response.json" => ("post", "/api/v1/gain", "200"),
        "batch_response.json" => ("post", "/api/v1/gain/batch", "200"),
        "heatmap_response.json" => ("post", "/api/v1/heatmap", "200"),
        "antenna_list_response.json" => ("get", "/api/v1/antennas", "200"),
        "antenna_details_response.json" => ("get", "/api/v1/antennas/{id}", "200"),
        "health_response.json" => ("get", "/health", "200"),
        "status_response.json" => ("get", "/status", "200"),
        "error_response.json" => ("post", "/api/v1/gain", "400"),
        other => panic!(
            "no endpoint mapping for examples/responses/{other} — add it to \
             response_file_endpoint"
        ),
    }
}

/// Named entries of `examples/api_requests.json` → endpoint and, for a
/// response, its status.
fn api_requests_endpoint(name: &str) -> (&'static str, &'static str, Option<&'static str>) {
    match name {
        "gain_request_ecef_quaternion" | "gain_request_geodetic_quaternion" => {
            ("post", "/api/v1/gain", None)
        }
        "gain_response" => ("post", "/api/v1/gain", Some("200")),
        "batch_request" => ("post", "/api/v1/gain/batch", None),
        "batch_response" => ("post", "/api/v1/gain/batch", Some("200")),
        "heatmap_request_rectangular" => ("post", "/api/v1/heatmap", None),
        "heatmap_response_rectangular" => ("post", "/api/v1/heatmap", Some("200")),
        "antenna_list_response" => ("get", "/api/v1/antennas", Some("200")),
        "antenna_details_response" => ("get", "/api/v1/antennas/{id}", Some("200")),
        "health_response" => ("get", "/health", Some("200")),
        "status_response" => ("get", "/status", Some("200")),
        // The error_response_* family are all `ErrorResponse` bodies. Each
        // documents a different failure, but the 400 response of
        // `POST /api/v1/gain` is the same schema every one of them must match.
        n if n.starts_with("error_response_") => ("post", "/api/v1/gain", Some("400")),
        other => panic!(
            "no endpoint mapping for example \"{other}\" in examples/api_requests.json — \
             add it to api_requests_endpoint"
        ),
    }
}

/// Resolves a concrete request URL against the spec's path templates,
/// returning the template it matches. Concrete ids stand in for `{param}`
/// segments, so this is a segment-wise match rather than a textual strip.
///
/// Panics if nothing matches — an example aimed at an endpoint the spec does
/// not document is exactly the drift this guard is for — and if more than one
/// template matches, since that would make the choice of schema arbitrary.
fn resolve_spec_path(spec: &Spec, method: &str, url: &str, origin: &str) -> (String, String) {
    let path = url
        .strip_prefix("{{baseUrl}}")
        .unwrap_or_else(|| panic!("{origin}: URL {url} does not start with {{{{baseUrl}}}}"));
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let method = method.to_ascii_lowercase();
    let wanted: Vec<&str> = path.trim_matches('/').split('/').collect();

    let paths = spec
        .root
        .get("paths")
        .and_then(Value::as_object)
        .expect("openapi.yaml has paths");
    let matches: Vec<&String> = paths
        .iter()
        .filter(|(template, item)| {
            if item.get(&method).is_none() {
                return false;
            }
            let segments: Vec<&str> = template.trim_matches('/').split('/').collect();
            segments.len() == wanted.len()
                && segments
                    .iter()
                    .zip(&wanted)
                    .all(|(t, w)| (t.starts_with('{') && t.ends_with('}')) || t == w)
        })
        .map(|(template, _)| template)
        .collect();

    match matches.as_slice() {
        [one] => (method, (*one).clone()),
        [] => panic!(
            "{origin}: sends {} {path}, which openapi.yaml documents no route for",
            method.to_uppercase()
        ),
        many => panic!(
            "{origin}: {} {path} matches {} spec path templates ({many:?}); the schema choice \
             would be arbitrary",
            method.to_uppercase(),
            many.len()
        ),
    }
}

fn json_files_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{} must exist: {e}", dir.display()))
        .map(|e| e.expect("readable dir entry").path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    files.sort();
    files
}

/// How many example bodies each source is known to contribute. A drop means
/// examples stopped being discovered, which would make the suite pass by
/// checking less; a rise means new ones arrived and should be reviewed. Update
/// these in the same commit that adds or removes an example.
const EXPECTED_FROM_REQUEST_FILES: usize = 10;
const EXPECTED_FROM_RESPONSE_FILES: usize = 8;
const EXPECTED_FROM_API_REQUESTS_JSON: usize = 16;
const EXPECTED_FROM_POSTMAN: usize = 4;
/// The spec's own inline `content.application/json.examples` bodies — the ones
/// Swagger UI and Redoc render, and the closest thing this repo has to a
/// reference client's view of the contract.
const EXPECTED_FROM_SPEC_INLINE: usize = 38;

fn collect_cases(spec: &Spec) -> Vec<Case> {
    let examples = repo_root().join("examples");
    let mut cases = Vec::new();

    for path in json_files_in(&examples.join("requests")) {
        let file = path
            .file_name()
            .and_then(|f| f.to_str())
            .expect("utf-8 name");
        let (method, endpoint) = request_file_endpoint(file);
        cases.push(Case::request(
            format!("examples/requests/{file}"),
            method,
            endpoint,
            read_json(&path),
        ));
    }
    assert_eq!(
        cases.len(),
        EXPECTED_FROM_REQUEST_FILES,
        "examples/requests contributed {} bodies, expected {EXPECTED_FROM_REQUEST_FILES}",
        cases.len()
    );

    let mark = cases.len();
    for path in json_files_in(&examples.join("responses")) {
        let file = path
            .file_name()
            .and_then(|f| f.to_str())
            .expect("utf-8 name");
        let (method, endpoint, status) = response_file_endpoint(file);
        cases.push(Case::response(
            format!("examples/responses/{file}"),
            method,
            endpoint,
            status,
            read_json(&path),
        ));
    }
    assert_eq!(
        cases.len() - mark,
        EXPECTED_FROM_RESPONSE_FILES,
        "examples/responses contributed {} bodies, expected {EXPECTED_FROM_RESPONSE_FILES}",
        cases.len() - mark
    );

    let mark = cases.len();
    let api_requests = read_json(&examples.join("api_requests.json"));
    let named = api_requests
        .get("examples")
        .and_then(Value::as_object)
        .expect("examples/api_requests.json has a top-level \"examples\" object");
    assert!(
        !named.is_empty(),
        "examples/api_requests.json \"examples\" map is empty — nothing would be checked"
    );
    for (name, entry) in named {
        let body = entry
            .get("request")
            .or_else(|| entry.get("response"))
            .unwrap_or_else(|| {
                panic!("example \"{name}\" has neither a \"request\" nor a \"response\" key")
            });
        let (method, endpoint, status) = api_requests_endpoint(name);
        // The file's own key names the direction; an example filed as a
        // "request" but mapped to a response status (or vice versa) is a
        // bookkeeping error in this test, not in the example.
        assert_eq!(
            entry.get("request").is_some(),
            status.is_none(),
            "example \"{name}\" is filed as a {} but mapped to a {}",
            if entry.get("request").is_some() {
                "request"
            } else {
                "response"
            },
            if status.is_none() {
                "request body"
            } else {
                "response body"
            }
        );
        let origin = format!("examples/api_requests.json::{name}");
        cases.push(match status {
            None => Case::request(origin, method, endpoint, body.clone()),
            Some(status) => Case::response(origin, method, endpoint, status, body.clone()),
        });
    }
    assert_eq!(
        cases.len() - mark,
        EXPECTED_FROM_API_REQUESTS_JSON,
        "examples/api_requests.json contributed {} bodies, expected \
         {EXPECTED_FROM_API_REQUESTS_JSON}",
        cases.len() - mark
    );

    let mark = cases.len();
    let postman = read_json(&examples.join("postman_collection.json"));
    collect_postman(spec, &postman, "", &mut cases);
    assert_eq!(
        cases.len() - mark,
        EXPECTED_FROM_POSTMAN,
        "examples/postman_collection.json contributed {} bodies, expected \
         {EXPECTED_FROM_POSTMAN}",
        cases.len() - mark
    );

    let mark = cases.len();
    collect_spec_inline_examples(spec, &mut cases);
    assert_eq!(
        cases.len() - mark,
        EXPECTED_FROM_SPEC_INLINE,
        "openapi.yaml's inline examples contributed {} bodies, expected \
         {EXPECTED_FROM_SPEC_INLINE}",
        cases.len() - mark
    );

    cases
}

/// Walks the postman collection's nested `item` folders.
///
/// **Every** request is resolved against the spec's path templates, not only
/// the ones carrying a body: the collection's seven GETs (`/health`, `/ready`,
/// `/status`, and the four antenna paths) have no body to validate, but a
/// C8-stage-4-style endpoint removal would leave one of them shipping a 404
/// with every other guard green. Resolution is the assertion for those;
/// bodied requests get their body validated on top.
fn collect_postman(spec: &Spec, node: &Value, prefix: &str, cases: &mut Vec<Case>) {
    let Some(items) = node.get("item").and_then(Value::as_array) else {
        return;
    };
    for item in items {
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("<unnamed>");
        if item.get("item").is_some() {
            collect_postman(spec, item, &format!("{prefix}{name}/"), cases);
            continue;
        }
        let Some(request) = item.get("request") else {
            continue;
        };
        let origin = format!("examples/postman_collection.json::{prefix}{name}");
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{origin} has no method"));
        let url = request
            .pointer("/url/raw")
            .or_else(|| request.get("url"))
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{origin} has no raw URL"));
        let (method, path) = resolve_spec_path(spec, method, url, &origin);

        let Some(raw) = request.pointer("/body/raw").and_then(Value::as_str) else {
            continue;
        };
        let body: Value = serde_json::from_str(raw)
            .unwrap_or_else(|e| panic!("{origin} has an unparseable JSON body: {e}"));
        cases.push(Case::request(origin, &method, &path, body));
    }
}

/// Collects the spec's own inline example bodies —
/// `content.application/json.examples.<name>.value` on request bodies and on
/// every documented response status.
///
/// These sit at exactly the `(method, path[, status])` positions this guard
/// already resolves schemas for, so each is validated against the schema
/// published beside it. An `examples` entry with no `value` (an
/// `externalValue`, say) panics rather than being skipped, and so does the
/// singular `example` form — neither appears today, and a silent skip is how a
/// surface stops being checked without anything saying so.
fn collect_spec_inline_examples(spec: &Spec, cases: &mut Vec<Case>) {
    let paths = spec
        .root
        .get("paths")
        .and_then(Value::as_object)
        .expect("openapi.yaml has paths");
    for (path, item) in paths {
        let operations = item.as_object().expect("path item is an object");
        for (method, op) in operations {
            if let Some(content) = op.pointer("/requestBody/content/application~1json") {
                push_inline(content, cases, path, method, None);
            }
            let Some(responses) = op.get("responses").and_then(Value::as_object) else {
                continue;
            };
            for (status, response) in responses {
                if let Some(content) = response.pointer("/content/application~1json") {
                    push_inline(content, cases, path, method, Some(status));
                }
            }
        }
    }
}

fn push_inline(
    content: &Value,
    cases: &mut Vec<Case>,
    path: &str,
    method: &str,
    status: Option<&str>,
) {
    let where_ = match status {
        None => format!("{} {path} request", method.to_uppercase()),
        Some(s) => format!("{} {path} {s} response", method.to_uppercase()),
    };
    assert!(
        content.get("example").is_none(),
        "{where_} uses the singular `example` key, which this guard does not collect — \
         teach collect_spec_inline_examples about it rather than leaving it unchecked"
    );
    let Some(examples) = content.get("examples") else {
        return;
    };
    let examples = examples
        .as_object()
        .unwrap_or_else(|| panic!("{where_}: `examples` is not an object"));
    for (name, entry) in examples {
        let body = entry.get("value").unwrap_or_else(|| {
            panic!(
                "{where_}: inline example \"{name}\" has no `value` (an `externalValue` cannot \
                 be validated here, and must not be silently skipped)"
            )
        });
        let origin = format!("openapi.yaml::{where_}::{name}");
        cases.push(match status {
            None => Case::request(origin, method, path, body.clone()),
            Some(status) => Case::response(origin, method, path, status, body.clone()),
        });
    }
}

fn schema_for<'a>(spec: &'a Spec, case: &Case) -> &'a Value {
    match &case.status {
        None => spec.request_schema(&case.method, &case.path),
        Some(status) => spec.response_schema(&case.method, &case.path, status),
    }
}

// ---------------------------------------------------------------------------
// The guards
// ---------------------------------------------------------------------------

/// Total example bodies this guard covers, as the sum of the per-source counts
/// above. Each source is also asserted individually in `collect_cases`, so a
/// surface that stops being discovered fails naming itself rather than showing
/// up as an off-by-N in one global number.
const EXPECTED_CASE_COUNT: usize = EXPECTED_FROM_REQUEST_FILES
    + EXPECTED_FROM_RESPONSE_FILES
    + EXPECTED_FROM_API_REQUESTS_JSON
    + EXPECTED_FROM_POSTMAN
    + EXPECTED_FROM_SPEC_INLINE;

#[test]
fn every_json_example_matches_its_published_schema() {
    let spec = Spec::load();
    let validator = Validator::new(&spec);
    let cases = collect_cases(&spec);

    assert_eq!(
        cases.len(),
        EXPECTED_CASE_COUNT,
        "expected to validate {EXPECTED_CASE_COUNT} examples, collected {} — if an example \
         was deliberately added or removed, update EXPECTED_CASE_COUNT in the same commit",
        cases.len()
    );

    let mut failures = Vec::new();
    for case in &cases {
        let report = validator.validate(schema_for(&spec, case), &case.body);
        if !report.ok() {
            let target = match &case.status {
                None => format!("{} {} request body", case.method.to_uppercase(), case.path),
                Some(s) => format!("{} {} {s} response", case.method.to_uppercase(), case.path),
            };
            failures.push(format!(
                "\n{} (validated against the {target} schema):\n  - {}",
                case.origin,
                report.errors.join("\n  - ")
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} examples disagree with the published openapi.yaml:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

/// Component schemas that no example reaches, each with the reason. This is an
/// inventory, not a waiver: a new component that nothing exercises fails the
/// test below until it is either given an example or listed here deliberately.
const UNEXERCISED_COMPONENTS: &[(&str, &str)] = &[
    (
        "H3LinkBudgetResponse",
        "no example response exists for POST /api/v1/h3-heatmap (the request half is covered \
         by examples/requests/h3_link_budget_request.json)",
    ),
    (
        "H3CellResult",
        "reachable only through H3LinkBudgetResponse",
    ),
    (
        "FeedListResponse",
        "no example response exists for GET /api/v1/antennas/{id}/feeds",
    ),
    (
        "GainError",
        "only appears on a FAILED item of a batch response; every batch example documents \
         successful items",
    ),
];

#[test]
fn every_component_schema_is_either_exercised_by_an_example_or_declared_uncovered() {
    let spec = Spec::load();
    let validator = Validator::new(&spec);

    let mut exercised = BTreeSet::new();
    for case in collect_cases(&spec) {
        let report = validator.validate(schema_for(&spec, &case), &case.body);
        // A failing example is the other test's business; its coverage still
        // counts, so that one broken example cannot cascade into a coverage
        // failure here.
        exercised.extend(report.exercised);
    }

    let declared: BTreeMap<&str, &str> = UNEXERCISED_COMPONENTS.iter().copied().collect();
    let all: BTreeSet<&str> = spec.components().keys().map(String::as_str).collect();

    let unexplained: Vec<&str> = all
        .iter()
        .copied()
        .filter(|c| !exercised.contains(*c) && !declared.contains_key(c))
        .collect();
    assert!(
        unexplained.is_empty(),
        "these published component schemas are exercised by no example and are not listed in \
         UNEXERCISED_COMPONENTS: {unexplained:?}. Add an example, or list the component with \
         the reason it has none."
    );

    let stale: Vec<&str> = declared
        .keys()
        .copied()
        .filter(|c| exercised.contains(*c))
        .collect();
    assert!(
        stale.is_empty(),
        "UNEXERCISED_COMPONENTS claims no example reaches {stale:?}, but one now does — \
         remove the stale entries so the list keeps meaning what it says."
    );

    let vanished: Vec<&str> = declared
        .keys()
        .copied()
        .filter(|c| !all.contains(*c))
        .collect();
    assert!(
        vanished.is_empty(),
        "UNEXERCISED_COMPONENTS names {vanished:?}, which the spec no longer declares."
    );
}

/// Anti-rot tripwire. A JSON Schema validator that ignores keywords it does not
/// know gets quietly weaker every time the spec grows a construct: it keeps
/// passing while asserting less. This walks every schema position in the spec
/// — not just the ones an example happens to reach — and fails on the first
/// keyword `Validator` does not implement.
#[test]
fn the_spec_uses_no_schema_keyword_this_validator_understands_nothing_of() {
    let spec = Spec::load();
    let mut found: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for (name, schema) in spec.components() {
        walk_schema(schema, &format!("components.schemas.{name}"), &mut found);
    }
    let paths = spec
        .root
        .get("paths")
        .and_then(Value::as_object)
        .expect("openapi.yaml has paths");
    for (path, item) in paths {
        let operations = item.as_object().expect("path item is an object");
        for (method, op) in operations {
            if let Some(s) = op.pointer("/requestBody/content/application~1json/schema") {
                walk_schema(s, &format!("{method} {path} request"), &mut found);
            }
            let Some(responses) = op.get("responses").and_then(Value::as_object) else {
                continue;
            };
            for (status, response) in responses {
                if let Some(s) = response.pointer("/content/application~1json/schema") {
                    walk_schema(s, &format!("{method} {path} {status}"), &mut found);
                }
            }
        }
    }

    assert!(
        found.is_empty(),
        "openapi.yaml uses schema keywords this validator does not implement, so \
         every_json_example_matches_its_published_schema is now asserting less than it \
         appears to. Implement each keyword in `Validator::check` and add it to \
         ENFORCED_KEYWORDS (or, only if it genuinely constrains nothing, to \
         ANNOTATION_KEYWORDS):\n{}",
        found
            .iter()
            .map(|(k, wheres)| format!("  `{k}` at {}", wheres.join(", ")))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Recursively records unknown keywords, descending only through positions
/// that hold a schema.
fn walk_schema(schema: &Value, at: &str, found: &mut BTreeMap<String, Vec<String>>) {
    let Some(obj) = schema.as_object() else {
        return; // boolean schema
    };
    for (key, value) in obj {
        if !keyword_is_known(key) {
            found.entry(key.clone()).or_default().push(at.to_string());
            continue;
        }
        match key.as_str() {
            "properties" => {
                if let Some(props) = value.as_object() {
                    for (name, sub) in props {
                        walk_schema(sub, &format!("{at}.{name}"), found);
                    }
                }
            }
            "items" => walk_schema(value, &format!("{at}[]"), found),
            "prefixItems" | "oneOf" => {
                if let Some(list) = value.as_array() {
                    for (i, sub) in list.iter().enumerate() {
                        walk_schema(sub, &format!("{at}/{key}[{i}]"), found);
                    }
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Negative controls
// ---------------------------------------------------------------------------

/// The unmutated fixture every negative control below starts from: a real
/// example, validated against the real published schema.
fn control_case() -> (Spec, Value, Value) {
    let spec = Spec::load();
    let body = read_json(&repo_root().join("examples/requests/gain_request.json"));
    let schema = spec.request_schema("post", "/api/v1/gain").clone();
    (spec, schema, body)
}

fn errors_for(spec: &Spec, schema: &Value, body: &Value) -> Vec<String> {
    Validator::new(spec).validate(schema, body).errors
}

/// Positive control. Without this, every assertion below could pass because
/// the validator rejects *everything*.
#[test]
fn the_unmutated_control_example_validates() {
    let (spec, schema, body) = control_case();
    let errors = errors_for(&spec, &schema, &body);
    assert!(
        errors.is_empty(),
        "the control example must validate cleanly or the negative controls prove nothing: \
         {errors:?}"
    );
}

#[test]
fn a_missing_required_field_is_caught() {
    let (spec, schema, mut body) = control_case();
    body.as_object_mut().expect("object").remove("antenna_id");
    let errors = errors_for(&spec, &schema, &body);
    assert!(
        errors.iter().any(|e| e.contains("requires `antenna_id`")),
        "dropping a required field must fail: {errors:?}"
    );
}

#[test]
fn a_wrong_scalar_type_is_caught() {
    let (spec, schema, mut body) = control_case();
    body["antenna_id"] = json!(17);
    let errors = errors_for(&spec, &schema, &body);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("antenna_id") && e.contains("declares type string")),
        "a number where the spec says string must fail: {errors:?}"
    );
}

#[test]
fn a_property_the_spec_does_not_declare_is_caught() {
    let (spec, schema, mut body) = control_case();
    // The exact shape of a half-applied rename: new name present, stale name
    // never deleted.
    body["feed_position"] = json!({"x": 1.0, "y": 2.0, "z": 3.0, "coordinate_system": "ecef"});
    let errors = errors_for(&spec, &schema, &body);
    assert!(
        errors.iter().any(|e| e.contains("`feed_position`")),
        "an undeclared property must fail: {errors:?}"
    );
}

#[test]
fn an_underscore_prefixed_annotation_key_is_exempt_but_nothing_else_is() {
    let (spec, schema, mut body) = control_case();
    body["_comment"] = json!("the examples' stand-in for a JSON comment");
    assert!(
        errors_for(&spec, &schema, &body).is_empty(),
        "`_`-prefixed annotation keys are the documented exemption"
    );
    // The exemption must be exactly that prefix, not "any key we have not
    // seen before".
    body.as_object_mut().expect("object").remove("_comment");
    body["comment"] = json!("no leading underscore");
    assert!(
        errors_for(&spec, &schema, &body)
            .iter()
            .any(|e| e.contains("`comment`")),
        "the exemption must not extend past the `_` prefix"
    );
}

#[test]
fn a_value_outside_a_closed_enum_is_caught() {
    let (spec, schema, mut body) = control_case();
    // The C8 stage 2 frame tag: the whole point is that it is a closed
    // vocabulary, and serde-level checks are the only other thing that says so.
    body["vehicle_position"]["coordinate_system"] = json!("eci");
    let errors = errors_for(&spec, &schema, &body);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("vehicle_position.coordinate_system") && e.contains("not one of")),
        "an unknown coordinate_system must fail: {errors:?}"
    );
}

#[test]
fn null_where_the_spec_forbids_it_is_caught() {
    let (spec, schema, mut body) = control_case();
    body["vehicle_position"]["x"] = Value::Null;
    let errors = errors_for(&spec, &schema, &body);
    assert!(
        errors.iter().any(|e| e.contains("vehicle_position.x")),
        "a null in a non-nullable number must fail: {errors:?}"
    );
    // …and the C7 `nan_as_null` fields, where null IS the contract, must not.
    let response_schema = spec.response_schema("post", "/api/v1/gain", "200");
    let mut response = read_json(&repo_root().join("examples/responses/gain_response.json"));
    response["gain_db"] = Value::Null;
    assert!(
        errors_for(&spec, response_schema, &response).is_empty(),
        "`gain_db: null` is the documented failed-evaluation sentinel and must validate"
    );
}

#[test]
fn a_number_below_the_spec_minimum_is_caught() {
    let spec = Spec::load();
    let schema = spec.response_schema("get", "/status", "200").clone();
    let mut body = read_json(&repo_root().join("examples/responses/status_response.json"));
    assert!(
        errors_for(&spec, &schema, &body).is_empty(),
        "control: the status example must validate before it is mutated"
    );
    body["uptime_seconds"] = json!(-1);
    let errors = errors_for(&spec, &schema, &body);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("uptime_seconds") && e.contains("minimum")),
        "a negative uptime must fail its `minimum: 0`: {errors:?}"
    );
}

#[test]
fn a_tuple_of_the_wrong_arity_is_caught() {
    let spec = Spec::load();
    let schema = spec
        .response_schema("get", "/api/v1/antennas/{id}", "200")
        .clone();
    let mut body = read_json(&repo_root().join("examples/responses/antenna_details_response.json"));
    assert!(
        errors_for(&spec, &schema, &body).is_empty(),
        "control: the antenna-details example must validate before it is mutated"
    );
    // `(min, max)` pairs are `prefixItems` + `items: false` — a third element
    // is exactly what that `false` exists to reject.
    body["validity_ranges"]["azimuth_deg"] = json!([0.0, 360.0, 720.0]);
    let errors = errors_for(&spec, &schema, &body);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("validity_ranges.azimuth_deg[2]")),
        "an over-long (min, max) tuple must fail: {errors:?}"
    );
}

#[test]
fn an_instance_matching_no_one_of_branch_is_caught() {
    let spec = Spec::load();
    let schema = spec.request_schema("post", "/api/v1/heatmap").clone();
    let mut body = read_json(&repo_root().join("examples/requests/heatmap_request.json"));
    assert!(
        errors_for(&spec, &schema, &body).is_empty(),
        "control: the heatmap example must validate before it is mutated"
    );
    // C8 stage 4 removed the `h3` variant; `GridConfig` is a single-variant
    // tagged enum, so this is the removed grid type coming back.
    body["grid_config"]["grid_type"] = json!("h3");
    let errors = errors_for(&spec, &schema, &body);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("grid_config") && e.contains("oneOf")),
        "a removed grid_type must match no `oneOf` branch: {errors:?}"
    );
}

#[test]
fn a_nested_underscore_key_is_not_exempt() {
    let (spec, schema, mut body) = control_case();
    // The annotation convention is a top-level one. A `_`-prefixed key deeper
    // in the tree is an undeclared property like any other — exempting it
    // would widen the hole silently.
    body["vehicle_position"]["_note"] = json!("not a top-level annotation");
    let errors = errors_for(&spec, &schema, &body);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("vehicle_position") && e.contains("`_note`")),
        "the `_` exemption must not reach past the root object: {errors:?}"
    );
}

#[test]
#[should_panic(expected = "`minimum` is not a number")]
fn a_malformed_minimum_panics_rather_than_vanishing() {
    let (spec, _, _) = control_case();
    // Every keyword position in this validator fails loudly on a shape it
    // cannot read. A `minimum` that silently stopped asserting because it
    // arrived as a string is the same silent-weakening class the unknown-
    // keyword tripwire exists for.
    let schema = json!({"type": "integer", "minimum": "0"});
    errors_for(&spec, &schema, &json!(-5));
}

#[test]
#[should_panic(expected = "`minItems` is not a non-negative integer")]
fn a_malformed_min_items_panics_rather_than_vanishing() {
    let (spec, _, _) = control_case();
    let schema = json!({"type": "array", "items": {"type": "number"}, "minItems": 2.5});
    errors_for(&spec, &schema, &json!([1.0]));
}

#[test]
fn postman_urls_resolve_against_the_spec_including_the_bodyless_gets() {
    let spec = Spec::load();
    // Concrete ids must land on the templated path…
    assert_eq!(
        resolve_spec_path(
            &spec,
            "GET",
            "{{baseUrl}}/api/v1/antennas/dsn_34m_uncalibrated/feeds/x_band",
            "control"
        ),
        (
            "get".to_string(),
            "/api/v1/antennas/{id}/feeds/{feed_id}".to_string()
        )
    );
    // …and a bare collection path must not be swallowed by the templated one.
    assert_eq!(
        resolve_spec_path(&spec, "GET", "{{baseUrl}}/api/v1/antennas", "control"),
        ("get".to_string(), "/api/v1/antennas".to_string())
    );
}

#[test]
#[should_panic(expected = "openapi.yaml documents no route for")]
fn a_postman_url_the_spec_does_not_document_is_caught() {
    // The C8 stage 4 shape: an endpoint is removed, and a collection entry is
    // left pointing at it. With no body there is nothing to validate, so
    // resolution is the only thing that can notice.
    let spec = Spec::load();
    resolve_spec_path(
        &spec,
        "GET",
        "{{baseUrl}}/api/v1/h3-heatmap-legacy",
        "control",
    );
}

#[test]
fn an_unimplemented_schema_keyword_is_a_failure_not_a_silent_skip() {
    let (spec, _, body) = control_case();
    // Stand-in for the real hazard: utoipa learns to emit a constraint, the
    // spec grows it, and a validator that shrugs at unknown keywords keeps
    // reporting success while checking less.
    let schema = json!({"type": "object", "patternProperties": {"^a": {"type": "string"}}});
    let errors = errors_for(&spec, &schema, &body);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("patternProperties") && e.contains("does not implement")),
        "an unknown keyword must be reported, never ignored: {errors:?}"
    );
}
