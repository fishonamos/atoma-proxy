use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::Value;
use tracing::{error, instrument};
use utoipa::OpenApi;

use crate::server::http_server::ProxyState;

use super::{authenticate_and_process, request_model::RequestModel};

pub struct RequestModelEmbeddings {
    model: String,
    input: String,
}

/// Path for the embeddings endpoint.
///
/// This endpoint follows the OpenAI API format for embeddings
/// and is used to generate vector embeddings for input text.
pub const EMBEDDINGS_PATH: &str = "/v1/embeddings";

// The default value when creating a new stack entry.
const STACK_ENTRY_COMPUTE_UNITS: u64 = 1000;
const STACK_ENTRY_PRICE: u64 = 100;

/// OpenAPI documentation for the embeddings endpoint.
#[derive(OpenApi)]
#[openapi(paths(embeddings_handler))]
pub(crate) struct EmbeddingsOpenApi;

impl RequestModel for RequestModelEmbeddings {
    fn new(request: &Value) -> Result<Self, StatusCode> {
        let model = request
            .get("model")
            .and_then(|m| m.as_str())
            .ok_or(StatusCode::BAD_REQUEST)?;
        let input = request
            .get("input")
            .and_then(|i| i.as_str())
            .ok_or(StatusCode::BAD_REQUEST)?;

        Ok(Self {
            model: model.to_string(),
            input: input.to_string(),
        })
    }

    fn get_model(&self) -> Result<String, StatusCode> {
        Ok(self.model.clone())
    }

    fn get_compute_units_estimate(&self, state: &ProxyState) -> Result<u64, StatusCode> {
        let tokenizer_index = state
            .models
            .iter()
            .position(|m| m == &self.model)
            .ok_or_else(|| {
                error!("Model not supported");
                StatusCode::BAD_REQUEST
            })?;
        let tokenizer = &state.tokenizers[tokenizer_index];

        let num_tokens = tokenizer
            .encode(self.input.as_str(), true)
            .map_err(|err| {
                error!("Failed to encode input: {:?}", err);
                StatusCode::INTERNAL_SERVER_ERROR
            })?
            .get_ids()
            .len() as u64;

        Ok(num_tokens)
    }
}

/// Handles the embeddings request.
///
/// This function processes embeddings requests by:
/// 1) Authenticating the request
/// 2) Getting the model from the payload
/// 3) Selecting an appropriate node/stack
/// 4) Forwarding the request to the selected node
/// 5) Returning the embeddings response
#[utoipa::path(
    post,
    path = EMBEDDINGS_PATH,
    responses(
        (status = OK, description = "Embeddings generated successfully", body = Value),
        (status = BAD_REQUEST, description = "Bad request"),
        (status = UNAUTHORIZED, description = "Unauthorized"),
        (status = INTERNAL_SERVER_ERROR, description = "Internal server error")
    )
)]
#[instrument(
    level = "info",
    skip_all,
    fields(endpoint = EMBEDDINGS_PATH, payload = ?payload)
)]
pub async fn embeddings_handler(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Result<Response<Body>, StatusCode> {
    let request_model = RequestModelEmbeddings::new(&payload)?;

    let (node_address, signature, selected_stack_small_id, headers, estimated_total_tokens) =
        authenticate_and_process(
            request_model,
            &state,
            headers,
            &payload,
            STACK_ENTRY_COMPUTE_UNITS,
            STACK_ENTRY_PRICE,
        )
        .await?;

    handle_embeddings_response(
        state,
        node_address,
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
        path = EMBEDDINGS_PATH,
        stack_small_id,
        estimated_total_tokens
    )
)]
async fn handle_embeddings_response(
    state: ProxyState,
    node_address: String,
    signature: String,
    selected_stack_small_id: i64,
    headers: HeaderMap,
    payload: Value,
    estimated_total_tokens: i64,
) -> Result<Response<Body>, StatusCode> {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}{}", node_address, EMBEDDINGS_PATH))
        .headers(headers)
        .header("X-Signature", signature)
        .header("X-Stack-Small-Id", selected_stack_small_id)
        .header("Content-Length", payload.to_string().len())
        .json(&payload)
        .send()
        .await
        .map_err(|err| {
            error!("Failed to send embeddings request: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .json::<Value>()
        .await
        .map_err(|err| {
            error!("Failed to parse embeddings response: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })
        .map(Json)?;

    // Extract the total tokens from the response
    let total_tokens = response
        .get("usage")
        .and_then(|usage| usage.get("total_tokens"))
        .and_then(|total_tokens| total_tokens.as_u64())
        .map(|n| n as i64)
        .unwrap_or(0);

    // Update the state manager with the actual token usage
    if let Err(e) = super::chat_completions::utils::update_state_manager(
        &state,
        selected_stack_small_id,
        estimated_total_tokens,
        total_tokens,
    )
    .await
    {
        error!("Error updating state manager: {}", e);
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(response.into_response())
}
