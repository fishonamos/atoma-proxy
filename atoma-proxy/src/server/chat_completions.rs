use std::sync::Arc;

use atoma_state::types::AtomaAtomaStateManagerEvent;
use axum::http::{header::AUTHORIZATION, StatusCode};
use axum::{extract::State, http::HeaderMap, Json};
use flume::Sender;
use serde_json::Value;
use tokenizers::Tokenizer;
use tokio::sync::{oneshot, RwLock};
use tracing::{error, instrument};
use utoipa::OpenApi;

use crate::sui::Sui;

use super::http_server::ProxyState;

/// Path for the chat completions endpoint.
///
/// This endpoint follows the OpenAI API format for chat completions
/// and is used to process chat-based requests for AI model inference.
pub const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";

// The default value when creating a new stack entry.
// TODO: Make this configurable or compute from the available stacks subscriptions.
const STACK_ENTRY_COMPUTE_UNITS: u64 = 1000;

// The default price for a new stack entry.
// TODO: Make this configurable or compute from the available stacks subscriptions.
const STACK_ENTRY_PRICE: u64 = 100;

/// OpenAPI documentation for the chat completions endpoint.
///
/// This struct is used to generate OpenAPI documentation for the chat completions
/// endpoint. It uses the `utoipa` crate's derive macro to automatically generate
/// the OpenAPI specification from the code.
#[derive(OpenApi)]
#[openapi(paths(chat_completions_handler))]
pub(crate) struct ChatCompletionsOpenApi;

/// Handles the chat completions request.
///
/// This function handles the chat completions request by processing the payload
/// and sending the request to the appropriate node for processing.
/// 1) Check the authentication of the request.
/// 2) Get the model from the payload.
/// 3) Get the stacks for the model.
/// 4) In case no stacks are found, get the tasks for the model and acquire a new stack entry.
/// 5) Get the public address of the selected node.
/// 6) Send the OpenAI API request to the selected node.
/// 7) Return the response from the OpenAI API.
///
/// # Arguments
///
/// * `state`: The shared state of the application.
/// * `headers`: The headers of the request.
/// * `payload`: The payload of the request.
///
/// # Returns
///
/// Returns the response from the OpenAI API or an error status code.
///
/// # Errors
///
/// Returns an error status code if the authentication fails, the model is not found, no tasks are found for the model, no node address is found, or an internal server error occurs.
#[utoipa::path(
    post,
    path = CHAT_COMPLETIONS_PATH,
    responses(
        (status = OK, description = "Chat completions", body = Value),
        (status = BAD_REQUEST, description = "Bad request"),
        (status = UNAUTHORIZED, description = "Unauthorized"),
        (status = INTERNAL_SERVER_ERROR, description = "Internal server error")
    )
)]
#[instrument(
    level = "info",
    skip_all,
    fields(endpoint = CHAT_COMPLETIONS_PATH, payload = ?payload)
)]
pub async fn chat_completions_handler(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    if !check_auth(&state.password, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Get the model from the payload, so we can find the task.
    let model = payload
        .get("model")
        .and_then(|model| model.as_str())
        .ok_or(StatusCode::BAD_REQUEST)?;

    let max_tokens = payload
        .get("max_tokens")
        .and_then(|max_tokens| max_tokens.as_u64())
        .ok_or(StatusCode::BAD_REQUEST)?;

    let messages = payload.get("messages").ok_or(StatusCode::BAD_REQUEST)?;

    let tokenizer_index = state
        .models
        .iter()
        .position(|m| m == model)
        .ok_or_else(|| {
            error!("Model not supported");
            StatusCode::BAD_REQUEST
        })?;

    let total_tokens =
        get_token_estimate(messages, max_tokens, &state.tokenizers[tokenizer_index]).await?;

    let (selected_stack_small_id, selected_node_id) =
        get_selected_node(model, &state.state_manager_sender, &state.sui, total_tokens).await?;

    let (result_sender, result_receiver) = oneshot::channel();
    state
        .state_manager_sender
        .send(AtomaAtomaStateManagerEvent::GetNodePublicAddress {
            node_small_id: selected_node_id,
            result_sender,
        })
        .map_err(|err| {
            error!("Failed to send GetNodePublicAddress event: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let node_address = result_receiver
        .await
        .map_err(|err| {
            error!("Failed to receive GetNodePublicAddress result: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .map_err(|err| {
            error!("Failed to get GetNodePublicAddress result: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or_else(|| {
            error!("No node address found for node {}", selected_node_id);
            StatusCode::NOT_FOUND
        })?;

    let signature = state
        .sui
        .write()
        .await
        .get_sui_signature(&payload)
        .map_err(|err| {
            error!("Failed to get Sui signature: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let mut headers = headers;
    headers.remove(AUTHORIZATION);

    let client = reqwest::blocking::Client::new();
    client
        .post(format!("{node_address}/v1/chat/completions"))
        .headers(headers)
        .header("X-Signature", signature)
        .header("X-Stack-Small-Id", selected_stack_small_id)
        .json(&payload)
        .send()
        .map_err(|err| {
            error!("Failed to send OpenAI API request: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })
        .map(|response| response.json::<Value>())?
        .map_err(|err| {
            error!("Failed to parse OpenAI API response: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })
        .map(Json)
}

/// Checks the authentication of the request.
///
/// This function checks the authentication of the request by comparing the
/// provided password with the `Authorization` header in the request.
///
/// # Arguments
///
/// * `password`: The password to check against.
/// * `headers`: The headers of the request.
///
/// # Returns
///
/// Returns `true` if the authentication is successful, `false` otherwise.
///
/// # Examples
///
/// ```rust
/// let mut headers = HeaderMap::new();
/// headers.insert("Authorization", "Bearer password".parse().unwrap());
/// let password = "password";
///
/// assert_eq!(check_auth(password, &headers), true);
/// assert_eq!(check_auth("wrong_password", &headers), false);
/// ```
#[instrument(level = "info", skip_all)]
fn check_auth(password: &str, headers: &HeaderMap) -> bool {
    if let Some(auth) = headers.get("Authorization") {
        if let Ok(auth) = auth.to_str() {
            if auth == format!("Bearer {}", password) {
                return true;
            }
        }
    }
    false
}

/// Estimates the total number of tokens in the messages.
///
/// This function estimates the total number of tokens in the messages by encoding each message
/// and counting the number of tokens in the encoded message.
///
/// # Arguments
///
/// * `messages`: The messages to estimate the number of tokens for.
/// * `max_tokens`: The maximum number of tokens allowed.
/// * `tokenizers`: The tokenizers used to encode the messages.
///
/// # Returns
///
/// Returns the estimated total number of tokens in the messages.
///
/// # Errors
///
/// Returns a bad request status code if the messages are not in the expected format.
/// Returns an internal server error status code if encoding the message fails.
#[instrument(level = "info", skip_all)]
async fn get_token_estimate(
    messages: &Value,
    max_tokens: u64,
    tokenizers: &Arc<Tokenizer>,
) -> Result<u64, StatusCode> {
    let mut total_num_tokens = 0;
    for message in messages.as_array().ok_or(StatusCode::BAD_REQUEST)? {
        let content = message
            .get("content")
            .and_then(|content| content.as_str())
            .ok_or(StatusCode::BAD_REQUEST)?;
        let num_tokens = tokenizers
            .encode(content, true)
            .map_err(|err| {
                error!("Failed to encode message: {:?}", err);
                StatusCode::INTERNAL_SERVER_ERROR
            })?
            .get_ids()
            .len() as u64;
        total_num_tokens += num_tokens;
        // add 2 tokens as a safety margin, for start and end message delimiters
        total_num_tokens += 2;
        // add 1 token as a safety margin, for the role name of the message
        total_num_tokens += 1;
    }
    total_num_tokens += max_tokens;
    Ok(total_num_tokens)
}

/// Selects a node for processing a model request by either finding an existing stack or acquiring a new one.
///
/// This function follows a two-step process:
/// 1. First, it attempts to find existing stacks that can handle the requested model and compute units
/// 2. If no suitable stacks exist, it acquires a new stack entry by:
///    - Finding available tasks for the model
///    - Creating a new stack entry with predefined compute units and price
///    - Registering the new stack with the state manager
///
/// # Arguments
///
/// * `model` - The name/identifier of the AI model being requested
/// * `state_manager_sender` - Channel for sending events to the state manager
/// * `sui` - Reference to the Sui interface for blockchain operations
/// * `total_tokens` - The total number of compute units (tokens) needed for the request
///
/// # Returns
///
/// Returns a tuple of `(stack_small_id, selected_node_id)` where:
/// * `stack_small_id` - The identifier for the selected/created stack
/// * `selected_node_id` - The identifier for the node that will process the request
///
/// # Errors
///
/// Returns a `StatusCode` error in the following cases:
/// * `INTERNAL_SERVER_ERROR` - Communication errors with state manager or Sui interface
/// * `NOT_FOUND` - No tasks available for the requested model
/// * `BAD_REQUEST` - Requested compute units exceed the maximum allowed limit
///
/// # Example
///
/// ```no_run
/// let (stack_id, node_id) = get_selected_node(
///     "gpt-4",
///     &state_manager_sender,
///     &sui,
///     1000
/// ).await?;
/// ```
#[instrument(level = "info", skip_all, fields(%model))]
async fn get_selected_node(
    model: &str,
    state_manager_sender: &Sender<AtomaAtomaStateManagerEvent>,
    sui: &Arc<RwLock<Sui>>,
    total_tokens: u64,
) -> Result<(i64, i64), StatusCode> {
    let (result_sender, result_receiver) = oneshot::channel();

    state_manager_sender
        .send(AtomaAtomaStateManagerEvent::GetStacksForModel {
            model: model.to_string(),
            free_compute_units: total_tokens as i64,
            result_sender,
        })
        .map_err(|err| {
            error!("Failed to send GetStacksForModel event: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let stacks = result_receiver
        .await
        .map_err(|err| {
            error!("Failed to receive GetStacksForModel result: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .map_err(|err| {
            error!("Failed to get GetStacksForModel result: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if stacks.is_empty() {
        let (result_sender, result_receiver) = oneshot::channel();
        state_manager_sender
            .send(AtomaAtomaStateManagerEvent::GetTasksForModel {
                model: model.to_string(),
                result_sender,
            })
            .map_err(|err| {
                error!("Failed to send GetTasksForModel event: {:?}", err);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
        let tasks = result_receiver
            .await
            .map_err(|err| {
                error!("Failed to receive GetTasksForModel result: {:?}", err);
                StatusCode::INTERNAL_SERVER_ERROR
            })?
            .map_err(|err| {
                error!("Failed to get GetTasksForModel result: {:?}", err);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
        if tasks.is_empty() {
            error!("No tasks found for model {}", model);
            return Err(StatusCode::NOT_FOUND);
        }
        // TODO: What should be the default values for the stack entry/price?
        if total_tokens > STACK_ENTRY_COMPUTE_UNITS {
            error!("Total tokens exceeds maximum limit of {STACK_ENTRY_COMPUTE_UNITS}");
            return Err(StatusCode::BAD_REQUEST);
        }
        let event = sui
            .write()
            .await
            .acquire_new_stack_entry(
                tasks[0].task_small_id as u64,
                STACK_ENTRY_COMPUTE_UNITS,
                STACK_ENTRY_PRICE,
            )
            .await
            .map_err(|err| {
                error!("Failed to acquire new stack entry: {:?}", err);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        let stack_small_id = event.stack_small_id.inner as i64;
        let selected_node_id = event.selected_node_id.inner as i64;

        // Send the NewStackAcquired event to the state manager, so we have it in the DB.
        state_manager_sender
            .send(AtomaAtomaStateManagerEvent::NewStackAcquired {
                event,
                already_computed_units: total_tokens as i64,
            })
            .map_err(|err| {
                error!("Failed to send NewStackAcquired event: {:?}", err);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        Ok((stack_small_id, selected_node_id))
    } else {
        Ok((stacks[0].stack_small_id, stacks[0].selected_node_id))
    }
}
