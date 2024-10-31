use anyhow::Result;
use blake2::{
    digest::generic_array::{typenum::U32, GenericArray},
    Blake2b, Digest,
};
use config::Config;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
use sui_keys::keystore::{AccountKeystore, Keystore};
use sui_sdk::{
    json::SuiJsonValue,
    rpc_types::Page,
    types::{
        base_types::{ObjectID, SuiAddress},
        crypto::EncodeDecodeBase64,
        SUI_RANDOMNESS_STATE_OBJECT_ID,
    },
    wallet_context::WalletContext,
    SuiClient,
};
use tracing::{info, instrument};

/// The Atoma's DB method name for acquiring a new stack entry
const ENDPOINT_NAME: &str = "acquire_new_stack_entry";
/// The Atoma's package ID on the Sui network
const ATOMA_PACKAGE_ID: &str = "0xc05bae323433740c969d8cf938c48d7559490be5f8dde158792e7a0623787013";
/// The Atoma's DB object ID on the Sui network
const ATOMA_DB_ID: &str = "0x7b8f40e38698deb650519a51f9c1a725bf8cfdc074d1552a4dc85976c2b414be";
/// The Atoma's TOMA token package ID on the Sui network
const TOMA_PACKAGE_ID: &str = "0xa1b2ce1a52abc52c5d21be9da0f752dbd587018bc666c5298f778f42de26df1d";
/// The Atoma's DB module name
const DB_MODULE_NAME: &str = "db";
/// The gas budget for the node registration transaction
const GAS_BUDGET: u64 = 5_000_000; // 0.005 SUI

/// A client for interacting with the Atoma network using the Sui blockchain.
///
/// The `AtomaProxy` struct provides methods to perform various operations
/// in the Atoma network, such as registering nodes, subscribing to models and tasks,
/// and managing transactions. It maintains a wallet context and optionally stores
/// a node badge representing the client's node registration status.
pub struct AtomaProxy {
    /// The wallet context used for managing blockchain interactions.
    wallet_ctx: WalletContext,

    /// The TOMA wallet object ID
    toma_wallet_id: Option<ObjectID>,

    /// The OpenAI API endpoint
    openai_api_endpoint: String,
}

impl AtomaProxy {
    /// Constructor
    pub async fn new(config: AtomaProxyConfig) -> Result<Self> {
        let sui_config_path = config.sui_config_path.clone();
        let sui_config_path = Path::new(&sui_config_path);
        let wallet_ctx = WalletContext::new(
            sui_config_path,
            config.request_timeout,
            config.max_concurrent_requests,
        )?;

        Ok(Self {
            wallet_ctx,
            toma_wallet_id: None,
            openai_api_endpoint: config.openai_api_endpoint,
        })
    }

    #[instrument(level = "info", skip_all, fields(
        endpoint = ENDPOINT_NAME,
        address = %self.wallet_ctx.active_address().unwrap()
    ))]
    pub async fn acquire_new_stack_entry(
        &mut self,
        task_small_id: u64,
        num_compute_units: u64,
        price: u64,
    ) -> Result<String> {
        let client = self.wallet_ctx.get_client().await?;
        let address = self.wallet_ctx.active_address()?;
        let toma_wallet_id = self.get_or_load_toma_wallet_object_id().await?;

        let tx = client
            .transaction_builder()
            .move_call(
                address,
                ATOMA_PACKAGE_ID.parse()?,
                DB_MODULE_NAME,
                ENDPOINT_NAME,
                vec![],
                vec![
                    SuiJsonValue::from_object_id(ATOMA_DB_ID.parse()?),
                    SuiJsonValue::from_object_id(toma_wallet_id),
                    SuiJsonValue::new(task_small_id.to_string().into())?,
                    SuiJsonValue::new(num_compute_units.to_string().into())?,
                    SuiJsonValue::new(price.to_string().into())?,
                    SuiJsonValue::from_object_id(SUI_RANDOMNESS_STATE_OBJECT_ID),
                ],
                None,
                GAS_BUDGET,
                None,
            )
            .await?;

        info!("Submitting acquire new stack entry transaction...");
        let tx = self.wallet_ctx.sign_transaction(&tx);
        let response = self.wallet_ctx.execute_transaction_must_succeed(tx).await;

        info!(
            "Acquire new stack entry transaction submitted successfully. Transaction digest: {:?}",
            response.digest
        );

        Ok(response.digest.to_string())
    }

    /// Get or load the TOMA wallet object ID
    ///
    /// This method checks if the TOMA wallet object ID is already loaded and returns it if so.
    /// Otherwise, it loads the TOMA wallet object ID by finding the most balance TOMA coin for the active address.
    ///
    /// # Returns
    ///
    /// Returns the TOMA wallet object ID.
    ///
    /// # Errors
    ///
    /// Returns an error if no TOMA wallet is found for the active address.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut client = AtomaProxy::new(config).await?;
    /// let toma_wallet_id = client.get_or_load_toma_wallet_object_id().await?;
    /// ```
    #[instrument(level = "info", skip_all, fields(
        endpoint = "get_or_load_toma_wallet_object_id",
        address = %self.wallet_ctx.active_address().unwrap()
    ))]
    pub async fn get_or_load_toma_wallet_object_id(&mut self) -> Result<ObjectID> {
        if let Some(toma_wallet_id) = self.toma_wallet_id {
            Ok(toma_wallet_id)
        } else {
            let active_address = self.wallet_ctx.active_address()?;
            let toma_wallet = find_toma_token_wallet(
                &self.wallet_ctx.get_client().await?,
                TOMA_PACKAGE_ID.parse()?,
                active_address,
            )
            .await;
            if let Ok(toma_wallet) = toma_wallet {
                self.toma_wallet_id = Some(toma_wallet);
                Ok(toma_wallet)
            } else {
                anyhow::bail!("No TOMA wallet found")
            }
        }
    }

    /// Send a request to an OpenAI API node
    ///
    /// # Arguments
    ///
    /// * `system_prompt` - The system prompt
    /// * `user_prompt` - The user prompt
    /// * `stream` - Whether to stream the response
    /// * `max_tokens` - The maximum number of tokens in the response
    ///
    /// # Returns
    ///
    /// Returns the response from the OpenAI API.
    ///
    /// # Errors
    ///
    /// Returns an error if the request to the OpenAI API fails.
    #[instrument(level = "info", skip_all, fields(
        endpoint = "send_openai_api_request",
        address = %self.wallet_ctx.active_address().unwrap()
    ))]
    pub async fn send_openai_api_request(
        &mut self,
        system_prompt: Option<String>,
        user_prompt: Option<String>,
        stream: Option<bool>,
        max_tokens: u64,
    ) -> Result<Value> {
        let active_address = self.wallet_ctx.active_address()?;
        let request = json!(
            {
                "model": "meta-llama/Llama-3.2-3B-Instruct", // hardcoded for now
                "messages": [
                    { "role": "system", "content": system_prompt },
                    { "role": "user", "content": user_prompt }
                ],
                "stream": stream,
                "max_tokens": max_tokens,
            }
        );
        let mut blake2b = Blake2b::new();
        blake2b.update(request.to_string().as_bytes());
        let hash: GenericArray<u8, U32> = blake2b.finalize();
        let signature = match &self.wallet_ctx.config.keystore {
            Keystore::File(keystore) => keystore.sign_hashed(&active_address, &hash)?,
            Keystore::InMem(keystore) => keystore.sign_hashed(&active_address, &hash)?,
        };
        let base64_signature = signature.encode_base64();

        let client = reqwest::Client::new();
        let response = client
            .post(&self.openai_api_endpoint)
            .header("X-Signature", base64_signature)
            .json(&request)
            .send()
            .await?;
        Ok(response.json::<Value>().await?)
    }
}

/// Find the TOMA token wallet for the given address
///
/// # Returns
///
/// Returns the TOMA token wallet object ID.
///
/// # Errors
///
/// Returns an error if no TOMA wallet is found for the active address.
#[instrument(level = "info", skip_all, fields(
    endpoint = "find_toma_token_wallet",
    address = %active_address
))]
async fn find_toma_token_wallet(
    client: &SuiClient,
    toma_package: ObjectID,
    active_address: SuiAddress,
) -> Result<ObjectID> {
    let Page { data: coins, .. } = client
        .coin_read_api()
        .get_coins(
            active_address,
            Some(format!("{toma_package}::toma::TOMA")),
            None,
            None,
        )
        .await?;
    coins
        .into_iter()
        .max_by_key(|coin| coin.balance)
        .map(|coin| coin.coin_object_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No TOMA coins for {active_address}. \
                Have you just received them? \
                It may take a few seconds for cache to refresh. \
                Double check that your address owns TOMA coins and try again."
            )
        })
}

/// Configuration for the Sui Event Subscriber
///
/// This struct holds the necessary configuration parameters for connecting to and
/// interacting with a Sui network, including URLs, package ID, timeout, and small IDs.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AtomaProxyConfig {
    /// The timeout duration for requests
    /// This sets the maximum time to wait for a response from the Sui network
    request_timeout: Option<Duration>,

    /// The maximum number of concurrent requests to the Sui client
    max_concurrent_requests: Option<u64>,

    /// Sui's config path
    sui_config_path: String,

    /// The OpenAI API endpoint
    openai_api_endpoint: String,
}

impl AtomaProxyConfig {
    /// Constructs a new `AtomaProxyConfig` instance from a configuration file path.
    ///
    /// # Arguments
    ///
    /// * `config_file_path` - A path-like object representing the location of the configuration file.
    ///
    /// # Returns
    ///
    /// Returns a new `AtomaSuiConfig` instance populated with values from the configuration file.
    ///
    /// # Panics
    ///
    /// This method will panic if:
    /// - The configuration file cannot be read or parsed.
    /// - The "atoma-sui" section is missing from the configuration file.
    /// - The configuration values cannot be deserialized into a `AtomaSuiConfig` instance.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use atoma_sui::config::AtomaSuiConfig;
    /// use std::path::Path;
    ///
    /// let config = AtomaSuiConfig::from_file_path("config.toml");
    /// ```
    pub fn from_file_path<P: AsRef<Path>>(config_file_path: P) -> Self {
        let builder = Config::builder().add_source(config::File::with_name(
            config_file_path.as_ref().to_str().unwrap(),
        ));
        let config = builder
            .build()
            .expect("Failed to generate atoma-sui configuration file");
        config
            .get::<Self>("atoma-sui")
            .expect("Failed to generate configuration instance")
    }
}
