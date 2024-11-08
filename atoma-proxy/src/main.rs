use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use atoma_state::{AtomaStateManager, AtomaStateManagerConfig};
use atoma_sui::AtomaSuiConfig;
use clap::Parser;
use futures::future::try_join_all;
use hf_hub::{api::sync::ApiBuilder, Repo, RepoType};
use server::{start_server, AtomaServiceConfig};
use sui::Sui;
use tokenizers::Tokenizer;
use tokio::{sync::watch, try_join};
use tracing::{error, instrument};
use tracing_appender::{
    non_blocking,
    rolling::{RollingFileAppender, Rotation},
};
use tracing_subscriber::{
    fmt::{self, format::FmtSpan, time::UtcTime},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter, Registry,
};

mod server;
mod sui;

/// The directory where the logs are stored.
const LOGS: &str = "./logs";
/// The log file name.
const LOG_FILE: &str = "atoma-proxy-service.log";

/// Command line arguments for the Atoma node
#[derive(Parser)]
struct Args {
    /// Path to the configuration file
    #[arg(short, long)]
    config_path: String,
}

/// Configuration for the Atoma proxy.
///
/// This struct holds the configuration settings for various components
/// of the Atoma proxy, including the Sui, service, and state manager configurations.
#[derive(Debug)]
struct Config {
    /// Configuration for the Sui component.
    sui: AtomaSuiConfig,

    /// Configuration for the service component.
    service: AtomaServiceConfig,

    /// Configuration for the state manager component.
    state: AtomaStateManagerConfig,
}

impl Config {
    async fn load(path: String) -> Self {
        Self {
            sui: AtomaSuiConfig::from_file_path(path.clone()),
            service: AtomaServiceConfig::from_file_path(path.clone()),
            state: AtomaStateManagerConfig::from_file_path(path),
        }
    }
}

/// Configure logging with JSON formatting, file output, and console output
fn setup_logging<P: AsRef<Path>>(log_dir: P) -> Result<()> {
    // Set up file appender with rotation
    let file_appender = RollingFileAppender::new(Rotation::DAILY, log_dir, LOG_FILE);

    // Create a non-blocking writer
    let (non_blocking_appender, _guard) = non_blocking(file_appender);

    // Create JSON formatter for file output
    let file_layer = fmt::layer()
        .json()
        .with_timer(UtcTime::rfc_3339())
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_target(true)
        .with_line_number(true)
        .with_file(true)
        .with_current_span(true)
        .with_span_list(true)
        .with_writer(non_blocking_appender);

    // Create console formatter for development
    let console_layer = fmt::layer()
        .pretty()
        .with_target(true)
        .with_thread_ids(true)
        .with_line_number(true)
        .with_file(true)
        .with_span_events(FmtSpan::ENTER);

    // Create filter from environment variable or default to info
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,atoma_node_service=debug"));

    // Combine layers with filter
    Registry::default()
        .with(env_filter)
        .with(console_layer)
        .with(file_layer)
        .init();

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    setup_logging(LOGS).context("Failed to setup logging")?;
    let args = Args::parse();
    let config = Config::load(args.config_path).await;

    let (_shutdown_sender, shutdown_receiver) = watch::channel(false);
    let (event_subscriber_sender, event_subscriber_receiver) = flume::unbounded();
    let (state_manager_sender, state_manager_receiver) = flume::unbounded();

    let sui_subscriber = atoma_sui::SuiEventSubscriber::new(
        config.sui.clone(),
        event_subscriber_sender,
        shutdown_receiver.clone(),
    );

    let sui = Sui::new(&config.sui).await?;

    // Initialize your StateManager here
    let state_manager = AtomaStateManager::new_from_url(
        config.state.database_url,
        event_subscriber_receiver,
        state_manager_receiver,
    )
    .await?;

    let shutdown_receiver_clone = shutdown_receiver.clone();
    let state_manager_handle = tokio::spawn(async move {
        Ok::<(), anyhow::Error>(state_manager.run(shutdown_receiver_clone).await?)
    });

    let sui_subscriber_handle =
        tokio::spawn(async move { Ok::<(), anyhow::Error>(sui_subscriber.run().await?) });
    let tokenizers = initialize_tokenizers(
        &config.service.models,
        &config.service.revisions,
        &config.service.hf_token,
    )
    .await?;
    let server_handle = tokio::spawn(async move {
        start_server(config.service, state_manager_sender, sui, tokenizers).await
    });

    let (sui_subscriber_result, server_result, state_manager_result) =
        try_join!(sui_subscriber_handle, server_handle, state_manager_handle)?;

    handle_tasks_results(sui_subscriber_result, state_manager_result, server_result)?;
    Ok(())
}

/// Initializes tokenizers for multiple models by fetching their configurations from HuggingFace.
///
/// This function concurrently fetches tokenizer configurations for multiple models from HuggingFace's
/// repository and initializes them. Each tokenizer is wrapped in an Arc for safe sharing across threads.
///
/// # Arguments
///
/// * `models` - A slice of model names/paths on HuggingFace (e.g., ["facebook/opt-125m"])
/// * `revisions` - A slice of revision/branch names corresponding to each model (e.g., ["main"])
/// * `hf_token` - The HuggingFace API token to use for fetching the tokenizer configurations
///
/// # Returns
///
/// Returns a `Result` containing a vector of Arc-wrapped tokenizers on success, or an error if:
/// - Failed to fetch tokenizer configuration from HuggingFace
/// - Failed to parse the tokenizer JSON
/// - Any other network or parsing errors occur
///
/// # Examples
///
/// ```rust,ignore
/// use anyhow::Result;
///
/// #[tokio::main]
/// async fn example() -> Result<()> {
///     let models = vec!["facebook/opt-125m".to_string()];
///     let revisions = vec!["main".to_string()];
///     
///     let tokenizers = initialize_tokenizers(&models, &revisions).await?;
///     Ok(())
/// }
/// ```
#[instrument(level = "info", skip(models, revisions))]
async fn initialize_tokenizers(
    models: &[String],
    revisions: &[String],
    hf_token: &String,
) -> Result<Vec<Arc<Tokenizer>>> {
    let api = ApiBuilder::new()
        .with_progress(true)
        .with_token(Some(hf_token.clone()))
        .build()?;
    let fetch_futures: Vec<_> = models
        .iter()
        .zip(revisions.iter())
        .map(|(model, revision)| {
            let api = api.clone();
            async move {
                let repo = api.repo(Repo::with_revision(
                    model.clone(),
                    RepoType::Model,
                    revision.clone(),
                ));

                let tokenizer_filename = repo
                    .get("tokenizer.json")
                    .expect("Failed to get tokenizer.json");

                Tokenizer::from_file(tokenizer_filename)
                    .map_err(|e| {
                        anyhow::anyhow!(format!(
                            "Failed to parse tokenizer for model {}, with error: {}",
                            model, e
                        ))
                    })
                    .map(Arc::new)
            }
        })
        .collect();

    try_join_all(fetch_futures).await
}

/// Handles the results of various tasks (subscriber, state manager, and server).
///
/// This function checks the results of the subscriber, state manager, and server tasks.
/// If any of the tasks return an error, it logs the error and returns it.
/// This is useful for ensuring that the application can gracefully handle failures
/// in any of its components and provide appropriate logging for debugging.
///
/// # Arguments
///
/// * `subscriber_result` - The result of the subscriber task, which may contain an error.
/// * `state_manager_result` - The result of the state manager task, which may contain an error.
/// * `server_result` - The result of the server task, which may contain an error.
///
/// # Returns
///
/// Returns a `Result<()>`, which is `Ok(())` if all tasks succeeded, or an error if any task failed.
#[instrument(level = "info", skip_all)]
fn handle_tasks_results(
    sui_subscriber_result: Result<()>,
    state_manager_result: Result<()>,
    server_result: Result<()>,
) -> Result<()> {
    let result_handler = |result: Result<()>, message: &str| {
        if let Err(e) = result {
            error!(
                target = "atoma-node-service",
                event = "atoma_node_service_shutdown",
                error = ?e,
                "{message}"
            );
            return Err(e);
        }
        Ok(())
    };
    result_handler(sui_subscriber_result, "Subscriber terminated abruptly")?;
    result_handler(state_manager_result, "State manager terminated abruptly")?;
    result_handler(server_result, "Server terminated abruptly")?;
    Ok(())
}
