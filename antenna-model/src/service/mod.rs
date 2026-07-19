//! Service Layer
//!
//! Business logic for antenna model computations.

pub mod batch;
pub mod cache;
pub use cache::{GainCache, GainCacheKey};
pub mod evaluator;
pub mod h3_link_budget;
pub mod heatmap;
#[cfg(test)]
pub(crate) mod test_support;
pub mod validator;

pub use batch::{evaluate_batch, evaluate_batch_with_budget};
pub use evaluator::{compute_gain_from_request, compute_gain_from_request_with_budget};
pub use h3_link_budget::{compute_h3_link_budget, compute_h3_link_budget_with_budget};
pub use heatmap::{generate_heatmap, generate_heatmap_with_budget};
