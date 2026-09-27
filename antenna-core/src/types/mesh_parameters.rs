use serde::{Deserialize, Serialize};

/// Wire-mesh reflector parameters.
///
/// Artifact invariants ([`crate::artifact`]): both dimensions positive, and the wire
/// thinner than the mesh spacing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeshParameters {
    /// Mesh spacing (hole size), millimetres.
    pub mesh_spacing_mm: f64,

    /// Wire diameter, millimetres.
    pub wire_diameter_mm: f64,
}

impl MeshParameters {
    /// Creates a new builder for constructing `MeshParameters`.
    pub fn builder() -> MeshParametersBuilder {
        MeshParametersBuilder::default()
    }
}

/// Builder for [`MeshParameters`]. Both fields are required.
#[derive(Default)]
pub struct MeshParametersBuilder {
    mesh_spacing_mm: Option<f64>,
    wire_diameter_mm: Option<f64>,
}

impl MeshParametersBuilder {
    pub fn mesh_spacing_mm(mut self, spacing: f64) -> Self {
        self.mesh_spacing_mm = Some(spacing);
        self
    }

    pub fn wire_diameter_mm(mut self, diameter: f64) -> Self {
        self.wire_diameter_mm = Some(diameter);
        self
    }

    pub fn build(self) -> Result<MeshParameters, String> {
        Ok(MeshParameters {
            mesh_spacing_mm: self.mesh_spacing_mm.ok_or("mesh_spacing_mm is required")?,
            wire_diameter_mm: self
                .wire_diameter_mm
                .ok_or("wire_diameter_mm is required")?,
        })
    }
}
