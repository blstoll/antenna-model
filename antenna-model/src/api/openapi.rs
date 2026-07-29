//! OpenAPI document definition: `ApiDoc` is the single source the committed
//! `openapi.yaml` is generated from.
//!
//! The spec file at the repository root is **generated, never hand-edited**.
//! `tests/openapi_spec.rs` asserts byte-for-byte equality between the committed
//! file and `ApiDoc::openapi().to_yaml()`, so after changing any request/response
//! schema, handler doc attribute, or this module, regenerate with
//!
//! ```text
//! cargo run -p antenna-model --bin generate_openapi
//! ```
//!
//! and review `git diff openapi.yaml` as a **contract change** before committing.
//!
//! Emission order is deterministic: utoipa's `preserve_order` /
//! `preserve_path_order` features make schema output follow declaration order and
//! path output follow the registration order in the `paths(...)` list below, and
//! the crate version is pinned exactly in `Cargo.toml` because any serialization
//! change in utoipa is a contract diff.

use utoipa::OpenApi;

/// The OpenAPI document root. The service-level description prose lives in
/// `openapi_info.md` (utoipa does not read this doc comment for `info`).
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Antenna Model Service API",
        version = "1.1.0",
        description = include_str!("openapi_info.md"),
        contact(name = "Antenna Model Service Support"),
        license(name = "Proprietary", url = "https://example.com/license")
    ),
    servers(
        (url = "http://localhost:3000", description = "Local development server"),
        (url = "http://antenna-model-service", description = "Kubernetes cluster internal service")
    ),
    tags(
        (name = "health", description = "Health check and service status endpoints"),
        (name = "gain", description = "Antenna gain computation endpoints"),
        (name = "heatmap", description = "Loss heatmap generation"),
        (name = "antennas", description = "Antenna and feed configuration queries")
    )
)]
pub struct ApiDoc;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_serializes_and_carries_the_info_block() {
        let doc = ApiDoc::openapi();
        let yaml = doc.to_yaml().expect("spec serializes to YAML");
        assert!(yaml.contains("title: Antenna Model Service API"));
        assert!(yaml.contains("version: 1.1.0"));
        assert!(yaml.contains("url: http://antenna-model-service"));
    }

    #[test]
    fn generation_is_deterministic_across_calls() {
        let a = ApiDoc::openapi().to_yaml().expect("first serialization");
        let b = ApiDoc::openapi().to_yaml().expect("second serialization");
        assert_eq!(a, b);
    }
}
