use std::sync::Arc;

use anyhow::Result;
use atoma_state::types::AtomaAtomaStateManagerEvent;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use flume::Sender;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    net::TcpListener,
    sync::{oneshot, RwLock},
};
use tracing::{error, instrument};

mod config;
pub use config::AtomaServiceConfig;

use crate::sui::Sui;

const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";
const NODE_PUBLIC_ADDRESS_REGISTRATION: &str = "/node/registration";

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
#[instrument(level = "info", skip_all, fields(endpoint = "check_auth"))]
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
#[instrument(level = "info", skip_all, fields(
    endpoint = "chat_completions_handler",
    payload = ?payload,
))]
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
        .map(|model| model.to_string())
        .ok_or(StatusCode::BAD_REQUEST)?;

    let selected_node_id =
        get_selected_node(model, &state.state_manager_sender, &state.sui).await?;

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

    let client = reqwest::Client::new();
    client
        .post(&node_address)
        .header("X-Signature", signature)
        .json(&payload)
        .send()
        .await
        .map_err(|err| {
            error!("Failed to send OpenAI API request: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })
        .map(|response| response.json::<Value>())?
        .await
        .map_err(|err| {
            error!("Failed to parse OpenAI API response: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })
        .map(Json)
}

#[instrument(level = "info", skip_all, fields(
    endpoint = "get_selected_node",
    model = %model,
))]
async fn get_selected_node(
    model: String,
    state_manager_sender: &Sender<AtomaAtomaStateManagerEvent>,
    sui: &Arc<RwLock<Sui>>,
) -> Result<i64, StatusCode> {
    let (result_sender, result_receiver) = oneshot::channel();

    state_manager_sender
        .send(AtomaAtomaStateManagerEvent::GetStacksForModel {
            model: model.clone(),
            free_compute_units: 1000,
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
                model: model.clone(),
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
        Ok(sui
            .write()
            .await
            .acquire_new_stack_entry(tasks[0].task_small_id as u64, 1000, 100)
            .await
            .map_err(|err| {
                error!("Failed to acquire new stack entry: {:?}", err);
                StatusCode::INTERNAL_SERVER_ERROR
            })? as i64)
    } else {
        Ok(stacks[0].selected_node_id)
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

#[instrument(level = "info", skip_all, fields(
    endpoint = "node_public_address_registration",
    payload = ?payload,
))]
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
#[instrument(level = "info", skip_all, fields(
    endpoint = "start_server",
    service_bind_address = %config.service_bind_address,
))]
pub async fn start_server(
    config: AtomaServiceConfig,
    state_manager_sender: Sender<AtomaAtomaStateManagerEvent>,
    sui: Sui,
) -> Result<()> {
    let tcp_listener = TcpListener::bind(config.service_bind_address).await?;

    let proxy_state = ProxyState {
        state_manager_sender,
        sui: Arc::new(RwLock::new(sui)),
        password: config.password,
    };
    let router = Router::new()
        .route(CHAT_COMPLETIONS_PATH, post(chat_completions_handler))
        .route(
            NODE_PUBLIC_ADDRESS_REGISTRATION,
            post(node_public_address_registration),
        )
        .with_state(proxy_state);
    let server = axum::serve(tcp_listener, router.into_make_service());
    server.await?;
    Ok(())
}
