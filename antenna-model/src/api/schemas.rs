//! API request and response schemas
//!
//! This module defines the data structures for API requests and responses,
//! all using serde for JSON serialization/deserialization.

use serde::{Deserialize, Serialize};

/// Status endpoint response
///
/// Returns the current health status of the service including uptime,
/// version, and operational status.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct StatusResponse {
    /// Service status - "ok" when operational
    pub status: String,
    /// Application version from Cargo.toml
    pub version: String,
    /// Uptime in seconds since server start
    pub uptime_seconds: u64,
}

impl StatusResponse {
    /// Create a new status response with "ok" status
    pub fn ok(version: String, uptime_seconds: u64) -> Self {
        Self {
            status: "ok".to_string(),
            version,
            uptime_seconds,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_response_serialization() {
        let response = StatusResponse::ok("0.1.0".to_string(), 3600);
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"status\":\"ok\""));
        assert!(json.contains("\"version\":\"0.1.0\""));
        assert!(json.contains("\"uptime_seconds\":3600"));
    }

    #[test]
    fn test_status_response_deserialization() {
        let json = r#"{"status":"ok","version":"0.1.0","uptime_seconds":3600}"#;
        let response: StatusResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.status, "ok");
        assert_eq!(response.version, "0.1.0");
        assert_eq!(response.uptime_seconds, 3600);
    }

    #[test]
    fn test_status_response_ok_constructor() {
        let response = StatusResponse::ok("1.2.3".to_string(), 7200);
        assert_eq!(response.status, "ok");
        assert_eq!(response.version, "1.2.3");
        assert_eq!(response.uptime_seconds, 7200);
    }
}
