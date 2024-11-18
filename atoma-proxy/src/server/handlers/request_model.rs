use axum::http::StatusCode;
use serde_json::Value;

use crate::server::http_server::ProxyState;

/// Trait for handling different types of AI model requests (chat, embeddings, images)
pub trait RequestModel {
    /// Creates a new request model from a JSON request
    fn new(request: &Value) -> Result<Self, StatusCode>
    where
        Self: Sized;

    /// Returns the model name
    fn get_model(&self) -> Result<String, StatusCode>;

    /// Estimates the compute units required for the request
    fn get_compute_units_estimate(&self, state: &ProxyState) -> Result<u64, StatusCode>;
}
