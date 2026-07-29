//! Route-coverage half of the C7 drift guard: the generated spec's
//! `(method, path)` set must equal the poem route table's.
//!
//! `tests/openapi_spec.rs` pins the committed file to `ApiDoc`; this test pins
//! `ApiDoc` to the routes actually registered in `api/routes.rs`. Paths in
//! `#[utoipa::path(...)]` are hand-typed (utoipa has no poem integration to
//! infer them), so this is what catches a handler routed at one path and
//! documented at another — the original C7 exit criterion, unchanged by the
//! 2026-07-28 re-scope to generation.
//!
//! The declared-route set is extracted by scanning the `routes.rs` source for
//! `.at("...", get/post(...))` registrations — the module's own convention
//! guarantees the route table lives in exactly one place. Source-scanning can
//! rot silently, so a **parser-honesty probe** backs it: every extracted route
//! is exercised through `poem::test::TestClient` with its declared method, and
//! must not answer with poem's bare route-miss 404 or method-mismatch 405.

use antenna_model::api::openapi::ApiDoc;
use antenna_model::api::routes::create_routes;
use antenna_model::api::AppState;
use poem::http::{Method, StatusCode};
use poem::{Endpoint, Request};
use std::collections::BTreeSet;
use std::sync::Arc;
use utoipa::OpenApi;

/// `(METHOD, path)` pairs the spec documents, path templates in `{param}` form.
fn spec_routes() -> BTreeSet<(String, String)> {
    let doc = ApiDoc::openapi();
    doc.paths
        .paths
        .iter()
        .flat_map(|(path, item)| {
            let mut methods = Vec::new();
            if item.get.is_some() {
                methods.push("GET");
            }
            if item.post.is_some() {
                methods.push("POST");
            }
            if item.put.is_some() {
                methods.push("PUT");
            }
            if item.delete.is_some() {
                methods.push("DELETE");
            }
            if item.patch.is_some() {
                methods.push("PATCH");
            }
            methods
                .into_iter()
                .map(move |m| (m.to_string(), path.clone()))
        })
        .collect()
}

/// `(METHOD, path)` pairs registered in `api/routes.rs`, poem `:param` segments
/// translated to `{param}`.
fn declared_routes() -> BTreeSet<(String, String)> {
    let src_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/api/routes.rs");
    let src = std::fs::read_to_string(&src_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", src_path.display()));

    // Collapse all whitespace so multi-line `.at(\n "path",\n heavy(post(...))`
    // registrations parse the same as single-line ones. Test modules also call
    // `.at(...)` on TestClient URLs? No — but keep the scan tolerant: only
    // `.at("` followed by a quoted path and a get(/post( within the same call
    // is taken as a registration.
    let compact: String = src.split_whitespace().collect::<Vec<_>>().join(" ");

    let mut routes = BTreeSet::new();
    for (idx, _) in compact.match_indices(".at(") {
        let rest = &compact[idx + ".at(".len()..];
        let rest = rest.trim_start();
        let Some(quoted) = rest.strip_prefix('"') else {
            continue;
        };
        let Some(end) = quoted.find('"') else {
            continue;
        };
        let path = &quoted[..end];
        if !path.starts_with('/') {
            continue;
        }
        // The method wrapper follows within the same registration; the window is
        // generous enough for `heavy(post(handlers::...))`.
        let after = &quoted[end..quoted.len().min(end + 60)];
        let method = if after.contains("get(") {
            "GET"
        } else if after.contains("post(") {
            "POST"
        } else {
            continue;
        };

        let template = path
            .split('/')
            .map(|seg| match seg.strip_prefix(':') {
                Some(name) => format!("{{{name}}}"),
                None => seg.to_string(),
            })
            .collect::<Vec<_>>()
            .join("/");
        routes.insert((method.to_string(), template));
    }
    routes
}

#[test]
fn spec_paths_equal_registered_routes() {
    let spec = spec_routes();
    let declared = declared_routes();

    for r in &declared {
        assert!(
            spec.contains(r),
            "poem registers `{} {}` but ApiDoc documents no such operation — add \
             #[utoipa::path] to the handler and register it in ApiDoc's paths(...)",
            r.0,
            r.1
        );
    }
    for r in &spec {
        assert!(
            declared.contains(r),
            "the spec documents `{} {}` but no poem route registers it — fix the \
             hand-typed path in the handler's #[utoipa::path] attribute",
            r.0,
            r.1
        );
    }
}

/// If the source scan ever misparses (a format change, macro indirection), it
/// must fail loudly here instead of silently shrinking the declared set.
#[tokio::test]
async fn declared_route_scan_is_honest() {
    let declared = declared_routes();
    assert!(
        !declared.is_empty(),
        "the routes.rs scan extracted zero routes — the parser has rotted"
    );

    let app = create_routes(Arc::new(AppState::with_defaults()));

    for (method, template) in &declared {
        // Dummy values for path params: the route must MATCH (anything but
        // poem's bare 404/405); a handler-level 404 carries a JSON error body
        // and is fine.
        let url = template.replace('{', "dummy_").replace('}', "");
        let req = Request::builder()
            .method(match method.as_str() {
                "GET" => Method::GET,
                "POST" => Method::POST,
                other => panic!("unhandled method {other} in probe"),
            })
            .uri(url.parse().expect("probe URL parses"))
            .finish();
        let resp = app.get_response(req).await;
        let status = resp.status();
        assert_ne!(
            status,
            StatusCode::METHOD_NOT_ALLOWED,
            "{method} {template}: route exists but not with this method — the scan \
             or the spec has the wrong verb"
        );
        if status == StatusCode::NOT_FOUND {
            // Distinguish a handler's typed 404 (JSON ErrorResponse body) from
            // poem's route-miss 404 (empty body).
            let body = resp.into_body().into_string().await.unwrap_or_default();
            assert!(
                !body.is_empty() && serde_json::from_str::<serde_json::Value>(&body).is_ok(),
                "{method} {template}: 404 with a non-JSON body — poem did not match \
                 the route the scan extracted (probe URL {url})"
            );
        }
    }
}
