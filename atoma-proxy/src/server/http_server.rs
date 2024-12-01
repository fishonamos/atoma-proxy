use std::sync::Arc;

use anyhow::Result;
use atoma_state::types::AtomaAtomaStateManagerEvent;
use axum::http::StatusCode;
use axum::middleware::from_fn_with_state;
use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use flume::Sender;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokenizers::Tokenizer;
use tokio::sync::watch;
use tokio::{net::TcpListener, sync::RwLock};
use tracing::{error, instrument};
use x25519_dalek::{PublicKey, SharedSecret, StaticSecret};
use zeroize::Zeroizing;

pub use components::openapi::openapi_routes;
use utoipa::{OpenApi, ToSchema};

use crate::server::handlers::{
    chat_completions::chat_completions_handler, chat_completions::CHAT_COMPLETIONS_PATH,
    embeddings::embeddings_handler, embeddings::EMBEDDINGS_PATH,
    image_generations::image_generations_handler, image_generations::IMAGE_GENERATIONS_PATH,
};
use crate::sui::Sui;

use super::components;
use super::handlers::chat_completions::CONFIDENTIAL_CHAT_COMPLETIONS_PATH;
use super::handlers::embeddings::CONFIDENTIAL_EMBEDDINGS_PATH;
use super::handlers::image_generations::CONFIDENTIAL_IMAGE_GENERATIONS_PATH;
use super::middleware::confidential_compute_middleware;
use super::AtomaServiceConfig;

/// Path for health check endpoint.
///
/// This endpoint is used to check the health of the atoma proxy service.
pub const HEALTH_PATH: &str = "/health";

/// Path for the models listing endpoint.
///
/// This endpoint follows the OpenAI API format and returns a list
/// of available AI models with their associated metadata and capabilities.
pub const MODELS_PATH: &str = "/v1/models";

/// Path for the node public address registration endpoint.
///
/// This endpoint is used to register or update the public address of a node
/// in the system, ensuring that the system has the correct address for routing requests.
pub const NODE_PUBLIC_ADDRESS_REGISTRATION_PATH: &str = "/node/registration";

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

    /// Secret key for X25519 key exchange.
    ///
    /// This key is used to compute shared secrets with nodes' public keys.
    /// The key is wrapped in both Arc (for shared ownership) and Zeroizing
    /// (to ensure the key material is securely erased from memory when dropped).
    secret_key: Arc<Zeroizing<StaticSecret>>,
}

impl ProxyState {
    /// Returns the public key for the X25519 key exchange.
    ///
    /// This key is used to compute shared secrets with nodes' public keys.
    pub fn public_key(&self) -> PublicKey {
        PublicKey::from(&**self.secret_key)
    }

    /// Computes the shared secret for the X25519 key exchange.
    ///
    /// This function computes the shared secret between the proxy's secret key
    /// and a given node's public key.
    pub fn compute_shared_secret(&self, public_key: &PublicKey) -> SharedSecret {
        self.secret_key.diffie_hellman(public_key)
    }
}

/// OpenAPI documentation for the models listing endpoint.
///
/// This struct is used to generate OpenAPI documentation for the models listing
/// endpoint. It uses the `utoipa` crate's derive macro to automatically generate
/// the OpenAPI specification from the code.
#[derive(OpenApi)]
#[openapi(paths(models_handler))]
pub(crate) struct ModelsOpenApi;

/// Handles requests to list available AI models.
///
/// This endpoint mimics the OpenAI models endpoint format, returning a list of
/// available models with their associated metadata and permissions. Each model
/// includes standard OpenAI-compatible fields to ensure compatibility with
/// existing OpenAI client libraries.
///
/// # Arguments
///
/// * `state` - The shared application state containing the list of available models
///
/// # Returns
///
/// Returns a JSON response containing:
/// * An "object" field set to "list"
/// * A "data" array containing model objects with the following fields:
///   - id: The model identifier
///   - object: Always set to "model"
///   - created: Timestamp (currently hardcoded)
///   - owned_by: Set to "atoma"
///   - root: Same as the model id
///   - parent: Set to null
///   - max_model_len: Maximum context length (currently hardcoded to 2048)
///   - permission: Array of permission objects describing model capabilities
///
/// # Example Response
///
/// ```json
/// {
///   "object": "list",
///   "data": [
///     {
///       "id": "meta-llama/Llama-3.1-70B-Instruct",
///       "object": "model",
///       "created": 1730930595,
///       "owned_by": "atoma",
///       "root": "meta-llama/Llama-3.1-70B-Instruct",
///       "parent": null,
///       "max_model_len": 2048,
///       "permission": [
///         {
///           "id": "modelperm-meta-llama/Llama-3.1-70B-Instruct",
///           "object": "model_permission",
///           "created": 1730930595,
///           "allow_create_engine": false,
///           "allow_sampling": true,
///           "allow_logprobs": true,
///           "allow_search_indices": false,
///           "allow_view": true,
///           "allow_fine_tuning": false,
///           "organization": "*",
///           "group": null,
///           "is_blocking": false
///         }
///       ]
///     }
///   ]
/// }
/// ```
#[utoipa::path(
    get,
    path = "",
    responses(
        (status = OK, description = "List of available models", body = Value),
        (status = INTERNAL_SERVER_ERROR, description = "Failed to retrieve list of available models")
    )
)]
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

/// Represents the payload for the node public address registration request.
///
/// This struct represents the payload for the node public address registration request.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct NodePublicAddressAssignment {
    /// Unique small integer identifier for the node
    node_small_id: u64,
    /// The public address of the node
    public_address: String,
}

#[derive(OpenApi)]
#[openapi(paths(node_public_address_registration))]
/// OpenAPI documentation for the node public address registration endpoint.
///
/// This struct is used to generate OpenAPI documentation for the node public address
/// registration endpoint. It uses the `utoipa` crate's derive macro to automatically
/// generate the OpenAPI specification from the code.
pub(crate) struct NodePublicAddressRegistrationOpenApi;

/// Handles the registration of a node's public address.
///
/// This endpoint allows nodes to register or update their public address in the system.
/// When a node comes online or changes its address, it can use this endpoint to ensure
/// the system has its current address for routing requests.
///
/// # Arguments
///
/// * `state` - The shared application state containing the state manager sender
/// * `payload` - The registration payload containing the node's ID and public address
///
/// # Returns
///
/// Returns `Ok(Json(Value::Null))` on successful registration, or an error status code
/// if the registration fails.
///
/// # Errors
///
/// Returns `StatusCode::INTERNAL_SERVER_ERROR` if:
/// * The state manager channel is closed
/// * The registration event cannot be sent
///
/// # Example Request Payload
///
/// ```json
/// {
///     "node_small_id": 123,
///     "public_address": "http://node-123.example.com:8080"
/// }
/// ```
#[utoipa::path(
    post,
    path = "",
    responses(
        (status = OK, description = "Node public address registered successfully", body = Value),
        (status = INTERNAL_SERVER_ERROR, description = "Failed to register node public address")
    )
)]
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

#[derive(OpenApi)]
#[openapi(paths(health))]
/// OpenAPI documentation for the health check endpoint.
///
/// This struct is used to generate OpenAPI documentation for the health check
/// endpoint. It uses the `utoipa` crate's derive macro to automatically generate
/// the OpenAPI specification from the code.
///
/// The health check endpoint is accessible at `/health` and returns a simple
/// JSON response indicating the service status.
pub(crate) struct HealthOpenApi;

/// Handles the health check request.
///
/// This endpoint is used to check the health of the atoma proxy service.
///
/// # Returns
///
/// Returns a JSON response with the status "ok".  
#[utoipa::path(
    get,
    path = "",
    responses(
        (status = OK, description = "Service is healthy", body = Value),
        (status = INTERNAL_SERVER_ERROR, description = "Service is unhealthy")
    )
)]
pub async fn health() -> Result<Json<Value>, StatusCode> {
    Ok(Json(json!({ "status": "ok" })))
}

/// Creates a router with the appropriate routes and state for the atoma proxy service.
///
/// This function sets up two sets of routes:
/// 1. Standard routes for public API endpoints
/// 2. Confidential routes for secure processing
///
/// # Routes
///
/// ## Standard Routes
/// - POST `/v1/chat/completions` - Chat completion endpoint
/// - POST `/v1/embeddings` - Text embedding generation
/// - POST `/v1/images/generations` - Image generation
/// - GET `/v1/models` - List available AI models
/// - POST `/node/registration` - Node public address registration
/// - GET `/health` - Service health check
/// - OpenAPI documentation routes
///
/// ## Confidential Routes
/// Secure variants of the processing endpoints:
/// - POST `/confidential/v1/chat/completions`
/// - POST `/confidential/v1/embeddings`
/// - POST `/confidential/v1/images/generations`
///
/// # Arguments
///
/// * `state` - Shared application state containing configuration and resources
///
/// # Returns
///
/// Returns an configured `Router` instance with all routes and middleware set up
pub fn create_router(state: ProxyState) -> Router {
    let confidential_router = Router::new()
        .route(
            CONFIDENTIAL_CHAT_COMPLETIONS_PATH,
            post(chat_completions_handler),
        )
        .route(CONFIDENTIAL_EMBEDDINGS_PATH, post(embeddings_handler))
        .route(
            CONFIDENTIAL_IMAGE_GENERATIONS_PATH,
            post(image_generations_handler),
        )
        .layer(from_fn_with_state(
            state.clone(),
            confidential_compute_middleware,
        ))
        .with_state(state.clone());

    Router::new()
        .route(CHAT_COMPLETIONS_PATH, post(chat_completions_handler))
        .route(EMBEDDINGS_PATH, post(embeddings_handler))
        .route(IMAGE_GENERATIONS_PATH, post(image_generations_handler))
        .route(MODELS_PATH, get(models_handler))
        .route(
            NODE_PUBLIC_ADDRESS_REGISTRATION_PATH,
            post(node_public_address_registration),
        )
        .with_state(state.clone())
        .route(HEALTH_PATH, get(health))
        .merge(openapi_routes())
        .merge(confidential_router)
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
    mut shutdown_receiver: watch::Receiver<bool>,
) -> Result<()> {
    let tcp_listener = TcpListener::bind(config.service_bind_address).await?;

    let secret_key = StaticSecret::random_from_rng(rand::thread_rng());
    let proxy_state = ProxyState {
        state_manager_sender,
        sui: Arc::new(RwLock::new(sui)),
        password: config.password,
        tokenizers: Arc::new(tokenizers),
        models: Arc::new(config.models),
        secret_key: Arc::new(Zeroizing::new(secret_key)),
    };
    let router = create_router(proxy_state);
    let server =
        axum::serve(tcp_listener, router.into_make_service()).with_graceful_shutdown(async move {
            shutdown_receiver
                .changed()
                .await
                .expect("Error receiving shutdown signal")
        });
    server.await?;
    Ok(())
}
