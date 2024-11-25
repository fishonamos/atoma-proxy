pub mod chat_completions;
pub mod embeddings;
pub mod image_generations;
pub mod request_model;
use crate::sui::Sui;
use crate::{server::http_server::ProxyState, sui::StackEntryResponse};
use atoma_state::types::AtomaAtomaStateManagerEvent;
use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode};
use flume::Sender;
use request_model::RequestModel;
use serde_json::Value;
use std::sync::Arc;
use sui_sdk::types::digests::TransactionDigest;
use tokio::sync::{oneshot, RwLock};
use tracing::{error, instrument};

/// Represents the processed and validated request data after authentication and initial processing.
///
/// This struct contains all the necessary information needed to forward a request to an inference node,
/// including authentication details, routing information, and request metadata.
#[derive(Debug)]
pub struct ProcessedRequest {
    /// The public address of the selected inference node
    pub node_address: String,
    /// The unique identifier for the selected node
    pub node_id: i64,
    /// The authentication signature for the request
    pub signature: String,
    /// The unique identifier for the selected stack
    pub stack_small_id: i64,
    /// HTTP headers to be forwarded with the request, excluding sensitive authentication headers
    pub headers: HeaderMap,
    /// The estimated number of compute units for this request (input + output)
    pub num_compute_units: u64,
    /// Optional transaction digest from stack entry creation, if a new stack was acquired
    pub tx_digest: Option<TransactionDigest>,
}

/// Authenticates the request and processes initial steps up to signature creation.
///
/// # Arguments
///
/// * `state` - The proxy state containing password, models, and other shared state
/// * `headers` - Request headers containing authorization
/// * `payload` - Request payload containing model and token information
///
/// # Returns
///
/// Returns a `ProcessedRequest` containing:
/// - `node_address`: Public address of the selected inference node
/// - `node_id`: Unique identifier for the selected node
/// - `signature`: Sui signature for request authentication
/// - `stack_small_id`: Identifier for the selected processing stack
/// - `headers`: Sanitized headers for forwarding (auth headers removed)
/// - `total_tokens`: Estimated total token usage
/// - `tx_digest`: Optional transaction digest if a new stack was created
///
/// # Errors
///
/// Returns `StatusCode` error in the following cases:
/// - `UNAUTHORIZED`: Invalid or missing authentication
/// - `BAD_REQUEST`: Invalid payload format or unsupported model
/// - `NOT_FOUND`: No available node address found
/// - `INTERNAL_SERVER_ERROR`: Various internal processing failures
#[instrument(level = "info", skip_all)]
async fn authenticate_and_process(
    request_model: impl RequestModel,
    state: &ProxyState,
    headers: HeaderMap,
    payload: &Value,
) -> Result<ProcessedRequest, StatusCode> {
    // Authenticate
    if !check_auth(&state.password, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Estimate compute units and the request model
    let model = request_model.get_model()?;
    let total_compute_units = request_model.get_compute_units_estimate(state)?;

    // Get node selection
    let SelectedNodeMetadata {
        stack_small_id: selected_stack_small_id,
        selected_node_id,
        tx_digest,
    } = get_selected_node(
        &model,
        &state.state_manager_sender,
        &state.sui,
        total_compute_units,
    )
    .await?;

    // Get node address
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

    // Get signature
    let signature = state
        .sui
        .write()
        .await
        .get_sui_signature(payload)
        .map_err(|err| {
            error!("Failed to get Sui signature: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Prepare headers
    let mut headers = headers;
    headers.remove(AUTHORIZATION);

    Ok(ProcessedRequest {
        node_address,
        node_id: selected_node_id,
        signature,
        stack_small_id: selected_stack_small_id,
        headers,
        num_compute_units: total_compute_units,
        tx_digest,
    })
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

/// Metadata returned when selecting a node for processing a model request
#[derive(Debug)]
pub struct SelectedNodeMetadata {
    /// The small ID of the stack
    pub stack_small_id: i64,
    /// The small ID of the selected node
    pub selected_node_id: i64,
    /// The transaction digest of the stack entry creation transaction
    pub tx_digest: Option<TransactionDigest>,
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
/// Returns a `SelectedNodeMetadata` containing:
/// * `stack_small_id` - The identifier for the selected/created stack
/// * `selected_node_id` - The identifier for the node that will process the request
/// * `tx_digest` - Optional transaction digest if a new stack was created
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
/// let metadata = get_selected_node(
///     "gpt-4",
///     &state_manager_sender,
///     &sui,
///     1000
/// ).await?;
/// println!("Selected stack ID: {}", metadata.stack_small_id);
/// ```
#[instrument(level = "info", skip_all, fields(%model))]
async fn get_selected_node(
    model: &str,
    state_manager_sender: &Sender<AtomaAtomaStateManagerEvent>,
    sui: &Arc<RwLock<Sui>>,
    total_tokens: u64,
) -> Result<SelectedNodeMetadata, StatusCode> {
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
            .send(AtomaAtomaStateManagerEvent::GetCheapestNodeForModel {
                model: model.to_string(),
                result_sender,
            })
            .map_err(|err| {
                error!("Failed to send GetTasksForModel event: {:?}", err);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
        let node = result_receiver
            .await
            .map_err(|err| {
                error!("Failed to receive GetTasksForModel result: {:?}", err);
                StatusCode::INTERNAL_SERVER_ERROR
            })?
            .map_err(|err| {
                error!("Failed to get GetTasksForModel result: {:?}", err);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
        let node: atoma_state::types::CheapestNode = match node {
            Some(node) => node,
            None => {
                error!("No tasks found for model {}", model);
                return Err(StatusCode::NOT_FOUND);
            }
        };
        let StackEntryResponse {
            transaction_digest: tx_digest,
            stack_created_event: event,
        } = sui
            .write()
            .await
            .acquire_new_stack_entry(
                node.task_small_id as u64,
                node.max_num_compute_units as u64,
                node.price_per_compute_unit as u64,
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

        Ok(SelectedNodeMetadata {
            stack_small_id,
            selected_node_id,
            tx_digest: Some(tx_digest),
        })
    } else {
        Ok(SelectedNodeMetadata {
            stack_small_id: stacks[0].stack_small_id,
            selected_node_id: stacks[0].selected_node_id,
            tx_digest: None,
        })
    }
}
