use atoma_state::types::Stack;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use tracing::{error, instrument};
use utoipa::OpenApi;

use crate::ProxyServiceState;

type Result<T> = std::result::Result<T, StatusCode>;

/// The path for the get_current_stacks endpoint.
pub(crate) const GET_STACKS_PATH: &str = "/stacks";

/// Returns a router with the stacks endpoint.
///
/// # Returns
/// * `Router<ProxyServiceState>` - A router with the stacks endpoint
pub(crate) fn stacks_router() -> Router<ProxyServiceState> {
    Router::new()
        .route(&format!("{GET_STACKS_PATH}/:id"), get(get_node_stacks))
        .route(GET_STACKS_PATH, get(get_current_stacks))
}

/// OpenAPI documentation for the get_current_stacks endpoint.
///
/// This struct is used to generate OpenAPI documentation for the get_current_stacks
/// endpoint. It uses the `utoipa` crate's derive macro to automatically generate
/// the OpenAPI specification from the code.
#[derive(OpenApi)]
#[openapi(paths(get_current_stacks))]
pub(crate) struct GetCurrentStacksOpenApi;

/// Retrieves all stacks that are not settled.
///
/// # Arguments
/// * `proxy_service_state` - The shared state containing the state manager
///
/// # Returns
/// * `Result<Json<Vec<Stack>>>` - A JSON response containing a list of stacks
///   - `Ok(Json<Vec<Stack>>)` - Successfully retrieved stacks
///   - `Err(StatusCode::INTERNAL_SERVER_ERROR)` - Failed to retrieve stacks from state manager
#[utoipa::path(
    get,
    path = "",
    responses(
        (status = OK, description = "Retrieves all stacks that are not settled"),
        (status = INTERNAL_SERVER_ERROR, description = "Failed to get all stacks")
    )
)]
#[instrument(level = "trace", skip_all)]
pub(crate) async fn get_current_stacks(
    State(proxy_service_state): State<ProxyServiceState>,
) -> Result<Json<Vec<Stack>>> {
    Ok(Json(
        proxy_service_state
            .atoma_state
            .get_current_stacks()
            .await
            .map_err(|_| {
                error!("Failed to get all stacks");
                StatusCode::INTERNAL_SERVER_ERROR
            })?,
    ))
}

/// Retrieves all stacks for a specific node identified by its small ID.
///
/// # Arguments
/// * `proxy_service_state` - The shared state containing the state manager
/// * `node_small_id` - The small ID of the node whose stacks should be retrieved
///
/// # Returns
/// * `Result<Json<Vec<Stack>>>` - A JSON response containing a list of stacks
///   - `Ok(Json<Vec<Stack>>)` - Successfully retrieved stacks
///   - `Err(StatusCode::INTERNAL_SERVER_ERROR)` - Failed to retrieve stacks from state manager
///
/// # Example Response
/// Returns a JSON array of Stack objects for the specified node
#[utoipa::path(
    get,
    path = "/{node_small_id}",
    params(
        ("node_small_id" = i64, description = "The small ID of the node whose stacks should be retrieved")
    ),
    responses(
        (status = OK, description = "Retrieves all stacks for a specific node"),
        (status = INTERNAL_SERVER_ERROR, description = "Failed to get node stack")
    )
)]
#[instrument(level = "trace", skip_all)]
pub(crate) async fn get_node_stacks(
    State(proxy_service_state): State<ProxyServiceState>,
    Path(node_small_id): Path<i64>,
) -> Result<Json<Vec<Stack>>> {
    Ok(Json(
        proxy_service_state
            .atoma_state
            .get_stack_by_id(node_small_id)
            .await
            .map_err(|_| {
                error!("Failed to get node stack");
                StatusCode::INTERNAL_SERVER_ERROR
            })?,
    ))
}
