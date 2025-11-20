//! Service Layer
//!
//! Business logic for antenna model computations.

pub mod batch;
pub mod evaluator;
pub mod heatmap;
pub mod validator;

pub use batch::evaluate_batch;
pub use evaluator::compute_gain_from_request;
pub use heatmap::generate_heatmap;
