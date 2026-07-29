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
    ),
    paths(
        crate::api::handlers::health,
        crate::api::handlers::ready,
        crate::api::handlers::status,
        crate::api::handlers::compute_gain,
        crate::api::handlers::compute_gain_batch,
        crate::api::handlers::generate_heatmap_endpoint,
        crate::api::handlers::h3_link_budget,
        crate::api::handlers::list_antennas,
        crate::api::handlers::get_antenna_details,
        crate::api::handlers::list_antenna_feeds,
        crate::api::handlers::get_feed_details,
    ),
    components(schemas(
        crate::api::schemas::CoordinateSystem,
        crate::api::schemas::Position3D,
        crate::api::schemas::Vector3D,
        crate::api::schemas::GainRequest,
        crate::api::schemas::GainResponse,
        crate::api::schemas::GeometryInfo,
        crate::api::schemas::ComputationMetadata,
        crate::api::schemas::BatchGainRequest,
        crate::api::schemas::BatchGainResponse,
        crate::api::schemas::BatchMetadata,
        crate::api::schemas::HeatmapRequest,
        crate::api::schemas::GridConfig,
        crate::api::schemas::RangeConfig,
        crate::api::schemas::HeatmapResponse,
        crate::api::schemas::GridData,
        crate::api::schemas::HeatmapMetadata,
        crate::api::schemas::H3LinkBudgetRequest,
        crate::api::schemas::H3CellResult,
        crate::api::schemas::H3LinkBudgetResponse,
        crate::api::schemas::AntennaListResponse,
        crate::api::schemas::AntennaInfo,
        crate::api::schemas::AntennaDetailsResponse,
        crate::api::schemas::FeedListResponse,
        crate::api::schemas::FeedInfo,
        crate::api::schemas::ValidityRangesInfo,
        crate::api::schemas::CalibrationInfo,
        crate::api::schemas::PhysicalParametersInfo,
        crate::api::schemas::MeshInfo,
        crate::api::schemas::CalibrationStatusInfo,
        crate::api::schemas::CoverageInfo,
        crate::api::schemas::HealthResponse,
        crate::api::schemas::StatusResponse,
        crate::warnings::WarningCode,
        crate::warnings::ApiWarning,
        crate::api::schemas::ErrorCode,
        crate::api::schemas::GainError,
        crate::api::schemas::ErrorResponse,
    ))
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
