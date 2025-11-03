//! Service Layer
//!
//! Business logic for antenna model computations.

pub mod batch;
pub mod evaluator;
pub mod validator;

pub use evaluator::compute_gain_from_request;
