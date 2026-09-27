use serde::{Deserialize, Serialize};

use super::{FeedParameters, MeshParameters, ReflectorGeometry};

/// Physical parameters of the physics model: reflector, feed, and optional mesh.
///
/// Every parameter `calibrate` fits against must be carried here, or the service
/// evaluates a different antenna than the residual surface describes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhysicalAntennaConfig {
    /// Reflector geometry.
    pub reflector: ReflectorGeometry,

    /// Feed parameters.
    pub feed: FeedParameters,

    /// Mesh parameters; `None` for a solid reflector.
    pub mesh: Option<MeshParameters>,
}

impl PhysicalAntennaConfig {
    /// Creates a new builder for constructing a `PhysicalAntennaConfig`.
    pub fn builder() -> PhysicalAntennaConfigBuilder {
        PhysicalAntennaConfigBuilder::default()
    }
}

/// Builder for [`PhysicalAntennaConfig`]. `reflector` and `feed` are required.
#[derive(Default)]
pub struct PhysicalAntennaConfigBuilder {
    reflector: Option<ReflectorGeometry>,
    feed: Option<FeedParameters>,
    mesh: Option<MeshParameters>,
}

impl PhysicalAntennaConfigBuilder {
    pub fn reflector(mut self, reflector: ReflectorGeometry) -> Self {
        self.reflector = Some(reflector);
        self
    }

    pub fn feed(mut self, feed: FeedParameters) -> Self {
        self.feed = Some(feed);
        self
    }

    pub fn mesh(mut self, mesh: MeshParameters) -> Self {
        self.mesh = Some(mesh);
        self
    }

    pub fn build(self) -> Result<PhysicalAntennaConfig, String> {
        Ok(PhysicalAntennaConfig {
            reflector: self.reflector.ok_or("reflector is required")?,
            feed: self.feed.ok_or("feed is required")?,
            mesh: self.mesh,
        })
    }
}
