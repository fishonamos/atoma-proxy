use axum::body::Body;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{extract::State, http::HeaderMap, Json};
use serde_json::Value;
use tracing::{error, instrument};
use utoipa::OpenApi;

use crate::server::http_server::ProxyState;

use super::authenticate_and_process;
use super::request_model::RequestModel;

pub struct RequestModelImageGenerations {
    model: String,
    n: u64,
    size: String,
}

/// Path for the image generations endpoint.
pub const IMAGE_GENERATIONS_PATH: &str = "/v1/images/generations";

// The default value when creating a new stack entry.
const STACK_ENTRY_COMPUTE_UNITS: u64 = 1000;

// The default price for a new stack entry.
const STACK_ENTRY_PRICE: u64 = 100;

/// OpenAPI documentation for the image generations endpoint.
#[derive(OpenApi)]
#[openapi(paths(image_generations_handler))]
pub(crate) struct ImageGenerationsOpenApi;

impl RequestModel for RequestModelImageGenerations {
    fn new(request: &Value) -> Result<Self, StatusCode> {
        let model = request
            .get("model")
            .and_then(|m| m.as_str())
            .ok_or(StatusCode::BAD_REQUEST)?;
        let n = request
            .get("n")
            .and_then(|n| n.as_u64())
            .ok_or(StatusCode::BAD_REQUEST)?;
        let size = request
            .get("size")
            .and_then(|s| s.as_str())
            .ok_or(StatusCode::BAD_REQUEST)?;

        Ok(Self {
            model: model.to_string(),
            n,
            size: size.to_string(),
        })
    }

    fn get_model(&self) -> Result<String, StatusCode> {
        Ok(self.model.clone())
    }

    fn get_compute_units_estimate(&self, _state: &ProxyState) -> Result<u64, StatusCode> {
        // Parse dimensions from size string (e.g., "1024x1024")
        let dimensions: Vec<u64> = self
            .size
            .split('x')
            .filter_map(|s| s.parse().ok())
            .collect();

        if dimensions.len() != 2 {
            error!("Invalid size format: {}", self.size);
            return Err(StatusCode::BAD_REQUEST);
        }

        let width = dimensions[0];
        let height = dimensions[1];

        // Calculate compute units based on number of images and pixel count
        Ok(self.n * width * height)
    }
}

/// Handles the image generations request.
#[utoipa::path(
    post,
    path = IMAGE_GENERATIONS_PATH,
    responses(
        (status = OK, description = "Image generations", body = Value),
        (status = BAD_REQUEST, description = "Bad request"),
        (status = UNAUTHORIZED, description = "Unauthorized"),
        (status = INTERNAL_SERVER_ERROR, description = "Internal server error")
    )
)]
#[instrument(
    level = "info",
    skip_all,
    fields(endpoint = IMAGE_GENERATIONS_PATH, payload = ?payload)
)]
pub async fn image_generations_handler(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Result<Response<Body>, StatusCode> {
    let request_model = RequestModelImageGenerations::new(&payload)?;

    let (
        node_address,
        node_id,
        signature,
        selected_stack_small_id,
        headers,
        estimated_total_tokens,
    ) = authenticate_and_process(
        request_model,
        &state,
        headers,
        &payload,
        STACK_ENTRY_COMPUTE_UNITS,
        STACK_ENTRY_PRICE,
    )
    .await?;

    handle_image_generation_response(
        state,
        node_address,
        node_id,
        signature,
        selected_stack_small_id,
        headers,
        payload,
        estimated_total_tokens as i64,
    )
    .await
}

#[instrument(
    level = "info",
    skip_all,
    fields(
        path = IMAGE_GENERATIONS_PATH,
        stack_small_id,
        estimated_total_tokens
    )
)]
async fn handle_image_generation_response(
    _state: ProxyState,
    node_address: String,
    node_id: i64,
    signature: String,
    selected_stack_small_id: i64,
    headers: HeaderMap,
    payload: Value,
    _estimated_total_tokens: i64,
) -> Result<Response<Body>, StatusCode> {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}{}", node_address, IMAGE_GENERATIONS_PATH))
        .headers(headers)
        .header("X-Signature", signature)
        .header("X-Stack-Small-Id", selected_stack_small_id)
        .header("Content-Length", payload.to_string().len())
        .json(&payload)
        .send()
        .await
        .map_err(|err| {
            error!("Failed to send image generation request: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .json::<Value>()
        .await
        .map_err(|err| {
            error!("Failed to parse image generation response: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })
        .map(Json)?;

    //TODO: Update node throughput performance

    Ok(response.into_response())
}
