//! Service Layer
//!
//! Business logic for antenna model computations.

pub mod batch;
pub mod cache;
pub use cache::{GainCache, GainCacheKey};
pub mod evaluator;
pub mod h3_link_budget;
pub mod heatmap;
pub mod validator;

pub use batch::evaluate_batch;
pub use evaluator::compute_gain_from_request;
pub use h3_link_budget::compute_h3_link_budget;
pub use heatmap::generate_heatmap;
