use std::sync::Arc;

use anyhow::Result;
use atoma_state::types::AtomaAtomaStateManagerEvent;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use flume::Sender;
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokenizers::Tokenizer;
use tokio::{
    net::TcpListener,
    sync::{oneshot, RwLock},
};
use tracing::{error, instrument};

mod config;
pub use config::AtomaServiceConfig;

use crate::sui::Sui;

const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";
const MODELS_PATH: &str = "/v1/models";
const NODE_PUBLIC_ADDRESS_REGISTRATION: &str = "/node/registration";
// The default value when creating a new stack entry.
// TODO: Make this configurable or compute from the available stacks subscriptions.
const STACK_ENTRY_COMPUTE_UNITS: u64 = 1000;
// The default price for a new stack entry.
// TODO: Make this configurable or compute from the available stacks subscriptions.
const STACK_ENTRY_PRICE: u64 = 100;

/// Represents the shared state of the application.
///
/// This struct holds various components and configurations that are shared
/// across different parts of the application, enabling efficient resource
/// management and communication between components.
#[derive(Clone)]
pub struct ProxyState {
    /// Channel sender for managing application events.
    ///
    /// This sender is used to communicate events and state changes to the
    /// state manager, allowing for efficient handling of application state
    /// updates and notifications across different components.
    pub state_manager_sender: Sender<AtomaAtomaStateManagerEvent>,

    /// `Sui` struct for handling Sui-related operations.
    ///
    /// This struct is used to interact with the Sui component of the application,
    /// enabling communication with the Sui service and handling Sui-related operations
    /// such as acquiring new stack entries.
    pub sui: Arc<RwLock<Sui>>,

    /// The password for the atoma proxy service.
    ///
    /// This password is used to authenticate requests to the atoma proxy service.
    pub password: String,

    /// Tokenizer used for processing text input.
    ///
    /// The tokenizer is responsible for breaking down text input into
    /// manageable tokens, which are then used in various natural language
    /// processing tasks.
    pub tokenizers: Arc<Vec<Arc<Tokenizer>>>,

    /// List of available AI models.
    ///
    /// This list contains the names or identifiers of AI models that
    /// the application can use for inference tasks. It allows the
    /// application to dynamically select and switch between different
    /// models as needed.
    pub models: Arc<Vec<String>>,
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
#[instrument(level = "info", skip_all, fields(?payload))]
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

    let estimated_total_tokens =
        get_token_estimate(messages, max_tokens, &state.tokenizers[tokenizer_index]).await?;

    let (selected_stack_small_id, selected_node_id) = get_selected_node(
        model,
        &state.state_manager_sender,
        &state.sui,
        estimated_total_tokens,
    )
    .await?;

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
    let response = client
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
        })?;
    let real_total_tokens = response
        .get("usage")
        .and_then(|usage| usage.get("total_tokens"))
        .and_then(|total_tokens| total_tokens.as_u64())
        .map(|n| n as i64)
        .unwrap_or(0);

    state
        .state_manager_sender
        .send(AtomaAtomaStateManagerEvent::UpdateStackNumTokens {
            stack_small_id: selected_stack_small_id,
            estimated_total_tokens: estimated_total_tokens as i64,
            total_tokens: real_total_tokens as i64,
        })
        .map_err(|err| {
            error!("Failed to send UpdateStackNumTokens event: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(response))
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

/// Represents the payload for the node public address registration request.
///
/// This struct represents the payload for the node public address registration request.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NodePublicAddressAssignment {
    /// Unique small integer identifier for the node
    node_small_id: u64,
    /// The public address of the node
    public_address: String,
}

async fn models_handler(State(state): State<ProxyState>) -> Result<Json<Value>, StatusCode> {
    // TODO: Implement proper model handling
    Ok(Json(json!({
        "object": "list",
        "data": state
        .models
        .iter()
        .map(|model| {
            json!({
              "id": model,
              "object": "model",
              "created": 1730930595,
              "owned_by": "atoma",
              "root": model,
              "parent": null,
              "max_model_len": 2048,
              "permission": [
                {
                  "id": format!("modelperm-{}", model),
                  "object": "model_permission",
                  "created": 1730930595,
                  "allow_create_engine": false,
                  "allow_sampling": true,
                  "allow_logprobs": true,
                  "allow_search_indices": false,
                  "allow_view": true,
                  "allow_fine_tuning": false,
                  "organization": "*",
                  "group": null,
                  "is_blocking": false
                }
              ]
            })
        })
        .collect::<Vec<_>>()
      }
    )))
}

#[instrument(level = "info", skip_all, fields(?payload))]
pub async fn node_public_address_registration(
    State(state): State<ProxyState>,
    Json(payload): Json<NodePublicAddressAssignment>,
) -> Result<Json<Value>, StatusCode> {
    state
        .state_manager_sender
        .send(AtomaAtomaStateManagerEvent::UpsertNodePublicAddress {
            node_small_id: payload.node_small_id as i64,
            public_address: payload.public_address.clone(),
        })
        .map_err(|err| {
            error!("Failed to send UpsertNodePublicAddress event: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(Value::Null))
}

/// Starts the atoma proxy server.
///
/// This function starts the atoma proxy server by binding to the specified address
/// and routing requests to the appropriate handlers.
///
/// # Arguments
///
/// * `config`: The configuration for the atoma proxy service.
/// * `state_manager_sender`: The sender channel for managing application events.
/// * `sui`: The Sui struct for handling Sui-related operations.
///
/// # Errors
///
/// Returns an error if the tcp listener fails to bind or the server fails to start.
#[instrument(level = "info", skip_all, fields(service_bind_address = %config.service_bind_address))]
pub async fn start_server(
    config: AtomaServiceConfig,
    state_manager_sender: Sender<AtomaAtomaStateManagerEvent>,
    sui: Sui,
    tokenizers: Vec<Arc<Tokenizer>>,
) -> Result<()> {
    let tcp_listener = TcpListener::bind(config.service_bind_address).await?;

    let proxy_state = ProxyState {
        state_manager_sender,
        sui: Arc::new(RwLock::new(sui)),
        password: config.password,
        tokenizers: Arc::new(tokenizers),
        models: Arc::new(config.models),
    };
    let router = Router::new()
        .route(CHAT_COMPLETIONS_PATH, post(chat_completions_handler))
        .route(MODELS_PATH, get(models_handler))
        .route(
            NODE_PUBLIC_ADDRESS_REGISTRATION,
            post(node_public_address_registration),
        )
        .with_state(proxy_state);
    let server = axum::serve(tcp_listener, router.into_make_service());
    server.await?;
    Ok(())
}
