use axum::{body::Body, extract::{Request, State}, middleware::Next, response::Response};
use reqwest::StatusCode;
use tracing::instrument;
use atoma_utils::encryption::{encrypt_plaintext, decrypt_cyphertext};

use super::http_server::ProxyState;

#[instrument(level = "trace", skip_all)]
pub async fn confidential_compute_middleware(
    state: State<ProxyState>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {


    Ok(next.run(req).await)
}
