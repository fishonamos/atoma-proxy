use atoma_state::{
    types::{NodeSubscription, Stack, Task},
    AtomaState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};

use tokio::{net::TcpListener, sync::watch::Receiver};
use tracing::{error, instrument};

type Result<T> = std::result::Result<T, StatusCode>;

/// State container for the Atoma daemon service that manages node operations and interactions.
///
/// The `DaemonState` struct serves as the central state management component for the Atoma daemon,
/// containing essential components for interacting with the Sui blockchain and managing node state.
/// It is designed to be shared across multiple request handlers and maintains thread-safe access
/// to shared resources.
///
/// # Thread Safety
///
/// This struct is designed to be safely shared across multiple threads:
/// - Implements `Clone` for easy sharing across request handlers
/// - Uses `Arc<RwLock>` for thread-safe access to the Sui client
/// - State manager and node badges vector use interior mutability patterns
///
/// # Example
///
/// ```rust,ignore
/// // Create a new daemon state instance
/// let daemon_state = DaemonState {
///     client: Arc::new(RwLock::new(AtomaSuiClient::new())),
///     state_manager: AtomaStateManager::new(),
///     node_badges: vec![(ObjectID::new([0; 32]), 1)],
/// };
///
/// // Clone the state for use in different handlers
/// let handler_state = daemon_state.clone();
/// ```
#[derive(Clone)]
pub struct DaemonState {
    /// Manages the persistent state of nodes, tasks, and other system components.
    /// Handles database operations and state synchronization.
    pub atoma_state: AtomaState,
}

/// Starts and runs the Atoma daemon service, handling HTTP requests and graceful shutdown.
/// This function initializes and runs the main daemon service that handles node operations,
///
/// # Arguments
///
/// * `daemon_state` - The shared state container for the daemon service, containing the Sui client,
///   state manager, and node badge information
/// * `tcp_listener` - A pre-configured TCP listener that the HTTP server will bind to
///
/// # Returns
///
/// * `anyhow::Result<()>` - Ok(()) on successful shutdown, or an error if
///   server initialization or shutdown fails
///
/// # Shutdown Behavior
///
/// The server implements graceful shutdown by:
/// 1. Listening for a Ctrl+C signal
/// 2. Logging shutdown initiation
/// 3. Waiting for existing connections to complete
///
/// # Example
///
/// ```rust,ignore
/// use tokio::net::TcpListener;
/// use tokio::sync::watch;
/// use atoma_daemon::{DaemonState, run_daemon};
///
/// async fn start_server() -> Result<(), Box<dyn std::error::Error>> {
///     let daemon_state = DaemonState::new(/* ... */);
///     let listener = TcpListener::bind("127.0.0.1:3000").await?;
///     
///     run_daemon(daemon_state, listener).await
/// }
/// ```
pub async fn run_daemon(
    daemon_state: DaemonState,
    tcp_listener: TcpListener,
    mut shutdown_receiver: Receiver<bool>,
) -> anyhow::Result<()> {
    let daemon_router = create_daemon_router(daemon_state);
    let server = axum::serve(tcp_listener, daemon_router.into_make_service())
        .with_graceful_shutdown(async move {
            shutdown_receiver
                .changed()
                .await
                .expect("Error receiving shutdown signal")
        });
    server.await?;
    Ok(())
}

/// Creates and configures the main router for the Atoma daemon HTTP API.
///
/// # Arguments
/// * `daemon_state` - The shared state container that will be available to all route handlers
///
/// # Returns
/// * `Router` - A configured axum Router instance with all API routes and shared state
///
/// # API Endpoints
///
/// ## Subscription Management
/// * `GET /subscriptions` - Get all subscriptions for registered nodes
/// * `GET /subscriptions/:id` - Get subscriptions for a specific node
/// * `POST /model_subscribe` - Subscribe a node to a model
/// * `POST /task_subscribe` - Subscribe a node to a task
/// * `POST /task_update_subscription` - Updates an already existing subscription to a task
/// * `POST /task_unsubscribe` - Unsubscribe a node from a task
///
/// ## Task Management
/// * `GET /tasks` - Get all available tasks
///
/// ## Stack Operations
/// * `GET /stacks` - Get all stacks for registered nodes
/// * `GET /stacks/:id` - Get stacks for a specific node
/// * `GET /almost_filled_stacks/:fraction` - Get stacks filled above specified fraction
/// * `GET /almost_filled_stacks/:id/:fraction` - Get node's stacks filled above fraction
/// * `GET /claimed_stacks` - Get all claimed stacks
/// * `GET /claimed_stacks/:id` - Get claimed stacks for a specific node
/// * `POST /try_settle_stack_ids` - Attempt to settle specified stacks
/// * `POST /submit_stack_settlement_attestations` - Submit attestations for stack settlement
/// * `POST /claim_funds` - Claim funds from completed stacks
///
/// ## Attestation Disputes
/// * `GET /against_attestation_disputes` - Get disputes against registered nodes
/// * `GET /against_attestation_disputes/:id` - Get disputes against a specific node
/// * `GET /own_attestation_disputes` - Get disputes initiated by registered nodes
/// * `GET /own_attestation_disputes/:id` - Get disputes initiated by a specific node
///
/// ## Node Registration
/// * `POST /register` - Register a new node
///
/// # Example
/// ```rust,ignore
/// use atoma_daemon::DaemonState;
///
/// let daemon_state = DaemonState::new(/* ... */);
/// let app = create_daemon_router(daemon_state);
/// // Start the server with the configured router
/// axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
///     .serve(app.into_make_service())
///     .await?;
/// ```
pub fn create_daemon_router(daemon_state: DaemonState) -> Router {
    Router::new()
        .route("/subscriptions/:id", get(get_node_subscriptions))
        .route("/tasks", get(get_all_tasks))
        .route("/task/:id", get(get_nodes_for_tasks))
        .route("/stacks/:id", get(get_node_stacks))
        .route("/get_stacks", get(get_current_stacks))
        .with_state(daemon_state)
        .route("/health", get(health))
}

/// Retrieves all stacks that are not settled.
///
/// # Arguments
/// * `daemon_state` - The shared state containing the state manager
///
/// # Returns
/// * `Result<Json<Vec<Stack>>>` - A JSON response containing a list of stacks
///   - `Ok(Json<Vec<Stack>>)` - Successfully retrieved stacks
///   - `Err(StatusCode::INTERNAL_SERVER_ERROR)` - Failed to retrieve stacks from state manager
async fn get_current_stacks(State(daemon_state): State<DaemonState>) -> Result<Json<Vec<Stack>>> {
    Ok(Json(
        daemon_state
            .atoma_state
            .get_current_stacks()
            .await
            .map_err(|_| {
                error!("Failed to get all stacks");
                StatusCode::INTERNAL_SERVER_ERROR
            })?,
    ))
}

/// Retrieves all subscriptions for a specific node identified by its small ID.
///
/// # Arguments
/// * `daemon_state` - The shared state containing the state manager
/// * `node_small_id` - The small ID of the node whose subscriptions should be retrieved
///
/// # Returns
/// * `Result<Json<Vec<NodeSubscription>>>` - A JSON response containing a list of subscriptions
///   - `Ok(Json<Vec<NodeSubscription>>)` - Successfully retrieved subscriptions
///   - `Err(StatusCode::INTERNAL_SERVER_ERROR)` - Failed to retrieve subscriptions from state manager
///
/// # Example Response
/// Returns a JSON array of NodeSubscription objects for the specified node, which may include:
/// ```json
/// [
///     {
///         "node_small_id": 123,
///         "model_name": "example_model",
///         "echelon_id": 1,
///         "subscription_time": "2024-03-21T12:00:00Z"
///     }
/// ]
/// ```
#[instrument(level = "trace", skip_all)]
async fn get_node_subscriptions(
    State(daemon_state): State<DaemonState>,
    Path(node_small_id): Path<i64>,
) -> Result<Json<Vec<NodeSubscription>>> {
    Ok(Json(
        daemon_state
            .atoma_state
            .get_all_node_subscriptions(&[node_small_id])
            .await
            .map_err(|_| {
                error!("Failed to get node subscriptions");
                StatusCode::INTERNAL_SERVER_ERROR
            })?,
    ))
}

/// Retrieves all subscriptions for a specific task identified by its task_id.
///
/// # Arguments
/// * `daemon_state` - The shared state containing the state manager
/// * `task_id` - The ID of the task whose subscriptions should be retrieved
///
/// # Returns
/// * `Result<Json<Vec<NodeSubscription>>` - A JSON response containing a list of subscriptions
///   - `Ok(Json<Vec<NodeSubscription>>)` - Successfully retrieved subscriptions
///   - `Err(StatusCode::INTERNAL_SERVER_ERROR)` - Failed to retrieve subscriptions from state manager
///
/// # Example Response
/// Returns a JSON array of NodeSubscription objects for the specified task, which may include:
/// ```json
/// [
///    {
///       "node_small_id": 123,
///       "model_name": "example_model",
///       "echelon_id": 1,
///       "subscription_time": "2024-03-21T12:00:00Z"
/// }
/// ]
/// ```
async fn get_nodes_for_tasks(
    State(daemon_state): State<DaemonState>,
    Path(task_id): Path<i64>,
) -> Result<Json<Vec<NodeSubscription>>> {
    Ok(Json(
        daemon_state
            .atoma_state
            .get_all_node_subscriptions_for_task(task_id)
            .await
            .map_err(|_| {
                error!("Failed to get node subscriptions");
                StatusCode::INTERNAL_SERVER_ERROR
            })?,
    ))
}
/// Retrieves all tasks from the state manager.
///
/// # Arguments
/// * `daemon_state` - The shared state containing the state manager
///
/// # Returns
/// * `Result<Json<Vec<Task>>>` - A JSON response containing a list of tasks
///   - `Ok(Json<Vec<Task>>)` - Successfully retrieved tasks
///   - `Err(StatusCode::INTERNAL_SERVER_ERROR)` - Failed to retrieve tasks from state manager
///
/// # Example Response
/// Returns a JSON array of Task objects representing all tasks in the system
#[instrument(level = "trace", skip_all)]
async fn get_all_tasks(State(daemon_state): State<DaemonState>) -> Result<Json<Vec<Task>>> {
    let all_tasks = daemon_state
        .atoma_state
        .get_all_tasks()
        .await
        .map_err(|_| {
            error!("Failed to get all tasks");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(all_tasks))
}

/// Health check endpoint for the daemon.
///
/// # Returns
/// * `StatusCode::OK` - Always returns OK
#[instrument(level = "trace", skip_all)]
async fn health() -> StatusCode {
    StatusCode::OK
}

/// Retrieves all stacks for a specific node identified by its small ID.
///
/// # Arguments
/// * `daemon_state` - The shared state containing the state manager
/// * `node_small_id` - The small ID of the node whose stacks should be retrieved
///
/// # Returns
/// * `Result<Json<Vec<Stack>>>` - A JSON response containing a list of stacks
///   - `Ok(Json<Vec<Stack>>)` - Successfully retrieved stacks
///   - `Err(StatusCode::INTERNAL_SERVER_ERROR)` - Failed to retrieve stacks from state manager
///
/// # Example Response
/// Returns a JSON array of Stack objects for the specified node
#[instrument(level = "trace", skip_all)]
async fn get_node_stacks(
    State(daemon_state): State<DaemonState>,
    Path(node_small_id): Path<i64>,
) -> Result<Json<Vec<Stack>>> {
    Ok(Json(
        daemon_state
            .atoma_state
            .get_stack_by_id(node_small_id)
            .await
            .map_err(|_| {
                error!("Failed to get node stack");
                StatusCode::INTERNAL_SERVER_ERROR
            })?,
    ))
}
