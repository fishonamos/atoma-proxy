use atoma_utils::encryption::decrypt_ciphertext;
use reqwest::StatusCode;
use serde_json::Value;
use tokio::sync::oneshot;
use tracing::{error, info, instrument};
use x25519_dalek::SharedSecret;

use super::middleware::NodeEncryptionMetadata;

pub mod chat_completions;
pub mod embeddings;
pub mod image_generations;
pub mod request_model;

#[instrument(
    level = "info",
    skip(response),
    fields(event = "extract-node-encryption-metadata")
)]
pub(crate) fn extract_node_encryption_metadata(
    response: Value,
) -> Result<NodeEncryptionMetadata, StatusCode> {
    let ciphertext = response
        .get("ciphertext")
        .and_then(|ciphertext| ciphertext.as_array())
        .ok_or_else(|| {
            error!("Failed to extract ciphertext from response: {:?}", response);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let ciphertext = ciphertext
        .iter()
        .map(|value| {
            value.as_u64().map(|value| value as u8).ok_or_else(|| {
                error!("Failed to extract ciphertext from response: {:?}", response);
                StatusCode::INTERNAL_SERVER_ERROR
            })
        })
        .collect::<Result<Vec<u8>, _>>()?;
    let nonce = response
        .get("nonce")
        .and_then(|nonce| nonce.as_array())
        .ok_or_else(|| {
            error!("Failed to extract nonce from response: {:?}", response);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let nonce = nonce
        .iter()
        .map(|value| {
            value.as_u64().map(|value| value as u8).ok_or_else(|| {
                error!("Failed to extract nonce from response: {:?}", response);
                StatusCode::INTERNAL_SERVER_ERROR
            })
        })
        .collect::<Result<Vec<u8>, _>>()?;
    Ok(NodeEncryptionMetadata { ciphertext, nonce })
}

#[instrument(
    level = "info",
    skip_all,
    fields(event = "confidential-compute-decryption-response")
)]
pub(crate) fn handle_confidential_compute_decryption_response(
    shared_secret: SharedSecret,
    ciphertext: &[u8],
    salt: &[u8],
    nonce: &[u8],
) -> Result<Value, StatusCode> {
    info!(
        target: "atoma-proxy-service",
        event = "confidential-compute-decryption-response",
        "Decrypting new response",
    );
    let (sender, receiver) = oneshot::channel();
    let plaintext_response_body_bytes = decrypt_ciphertext(shared_secret, ciphertext, salt, nonce);
    let response_body = serde_json::from_slice(&plaintext_response_body_bytes).map_err(|_| {
        error!("Failed to parse response body as JSON");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(response_body)
}
