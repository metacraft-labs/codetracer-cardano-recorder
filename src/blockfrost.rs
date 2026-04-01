//! Blockfrost API client for fetching Cardano transaction data.
//!
//! Uses the [Blockfrost](https://blockfrost.io/) REST API to retrieve
//! transaction details, extract Plutus script bytes, datum, redeemer,
//! and script context needed to replay validator execution.

use eyre::{eyre, Result};

use crate::transaction::{PlutusVersion, TransactionData};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Environment variable name for the Blockfrost API key.
pub const BLOCKFROST_API_KEY_ENV: &str = "BLOCKFROST_API_KEY";

/// Default Blockfrost API base URL (mainnet).
pub const DEFAULT_BASE_URL: &str = "https://cardano-mainnet.blockfrost.io/api/v0";

// ---------------------------------------------------------------------------
// BlockfrostClient
// ---------------------------------------------------------------------------

/// A lightweight client for the Blockfrost Cardano API.
///
/// Provides methods to fetch transaction data needed for on-chain replay.
#[derive(Debug, Clone)]
pub struct BlockfrostClient {
    /// Blockfrost project API key.
    pub api_key: String,

    /// Base URL for Blockfrost API requests.
    pub base_url: String,
}

impl BlockfrostClient {
    /// Create a new client with the given API key and default base URL.
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: DEFAULT_BASE_URL.to_string(),
        }
    }

    /// Create a new client with a custom base URL.
    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        Self { api_key, base_url }
    }

    /// Try to create a client from the `BLOCKFROST_API_KEY` environment variable.
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var(BLOCKFROST_API_KEY_ENV).map_err(|_| {
            eyre!(
                "Blockfrost API key not configured. Set the {BLOCKFROST_API_KEY_ENV} \
                 environment variable or pass --blockfrost-key on the command line."
            )
        })?;
        Ok(Self::new(api_key))
    }

    /// Fetch transaction data for the given transaction hash.
    ///
    /// This retrieves the Plutus script, datum, redeemer, and script context
    /// from the Blockfrost API and assembles them into a `TransactionData`
    /// struct suitable for replay.
    ///
    /// # Current limitations
    ///
    /// This is a placeholder implementation. A full implementation would:
    /// 1. GET `/txs/{hash}/utxos` to find script inputs and their datum hashes
    /// 2. GET `/txs/{hash}/redeemers` to get the redeemer data
    /// 3. GET `/scripts/{script_hash}/cbor` to get the script bytes
    /// 4. GET `/scripts/datum/{datum_hash}/cbor` to get the datum
    /// 5. Reconstruct the ScriptContext from the transaction data
    pub fn fetch_transaction(&self, tx_hash: &str) -> Result<TransactionData> {
        if self.api_key.is_empty() {
            return Err(eyre!(
                "Blockfrost API key is empty. Set {BLOCKFROST_API_KEY_ENV} or pass \
                 --blockfrost-key."
            ));
        }

        // Validate transaction hash format (64 hex characters).
        if tx_hash.len() != 64 || !tx_hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(eyre!(
                "invalid transaction hash: expected 64 hex characters, got '{tx_hash}'"
            ));
        }

        // Fetch transaction UTXOs to find script inputs.
        let utxos_url = format!("{}/txs/{}/utxos", self.base_url, tx_hash);
        let mut utxos_resp = ureq::get(&utxos_url)
            .header("project_id", &self.api_key)
            .call()
            .map_err(|e| eyre!("failed to fetch transaction UTXOs: {e}"))?;

        let utxos_json: serde_json::Value = utxos_resp
            .body_mut()
            .read_json()
            .map_err(|e| eyre!("failed to parse UTXOs response: {e}"))?;

        // Find the first input that has a script reference or inline datum.
        let inputs = utxos_json["inputs"]
            .as_array()
            .ok_or_else(|| eyre!("no inputs found in transaction UTXOs"))?;

        // Look for a script hash in the inputs.
        let script_hash = inputs
            .iter()
            .find_map(|input| input["reference_script_hash"].as_str())
            .or_else(|| {
                utxos_json["outputs"].as_array().and_then(|outputs| {
                    outputs
                        .iter()
                        .find_map(|output| output["reference_script_hash"].as_str())
                })
            });

        let script_hash =
            script_hash.ok_or_else(|| eyre!("no Plutus script found in transaction {tx_hash}"))?;

        // Fetch the script CBOR.
        let script_url = format!("{}/scripts/{}/cbor", self.base_url, script_hash);
        let mut script_resp = ureq::get(&script_url)
            .header("project_id", &self.api_key)
            .call()
            .map_err(|e| eyre!("failed to fetch script CBOR: {e}"))?;

        let script_json: serde_json::Value = script_resp
            .body_mut()
            .read_json()
            .map_err(|e| eyre!("failed to parse script response: {e}"))?;

        let script_hex = script_json["cbor"]
            .as_str()
            .ok_or_else(|| eyre!("no CBOR field in script response"))?;

        let script_bytes =
            hex::decode(script_hex).map_err(|e| eyre!("invalid hex in script CBOR: {e}"))?;

        // Fetch redeemers.
        let redeemers_url = format!("{}/txs/{}/redeemers", self.base_url, tx_hash);
        let mut redeemers_resp = ureq::get(&redeemers_url)
            .header("project_id", &self.api_key)
            .call()
            .map_err(|e| eyre!("failed to fetch redeemers: {e}"))?;

        let redeemers_json: serde_json::Value = redeemers_resp
            .body_mut()
            .read_json()
            .map_err(|e| eyre!("failed to parse redeemers response: {e}"))?;

        let redeemers = redeemers_json
            .as_array()
            .ok_or_else(|| eyre!("no redeemers found in transaction"))?;

        let redeemer_entry = redeemers
            .first()
            .ok_or_else(|| eyre!("empty redeemers list"))?;

        let redeemer_hex = redeemer_entry["redeemer_data_hash"]
            .as_str()
            .unwrap_or_default()
            .to_string();

        // Determine Plutus version from script type.
        let script_type_url = format!("{}/scripts/{}", self.base_url, script_hash);
        let mut script_type_resp = ureq::get(&script_type_url)
            .header("project_id", &self.api_key)
            .call()
            .map_err(|e| eyre!("failed to fetch script type: {e}"))?;

        let script_type_json: serde_json::Value = script_type_resp
            .body_mut()
            .read_json()
            .map_err(|e| eyre!("failed to parse script type response: {e}"))?;

        let script_version = match script_type_json["type"].as_str() {
            Some("plutusV1") => PlutusVersion::V1,
            Some("plutusV2") => PlutusVersion::V2,
            Some("plutusV3") => PlutusVersion::V3,
            Some(other) => return Err(eyre!("unsupported script type: {other}")),
            None => return Err(eyre!("could not determine script type")),
        };

        // Look for datum in inputs.
        let datum = inputs
            .iter()
            .find_map(|input| input["inline_datum"].as_str())
            .map(|s| s.to_string());

        // Note: A complete implementation would also reconstruct the
        // ScriptContext from the full transaction data. For now, we return
        // the redeemer data hash as a placeholder.
        Ok(TransactionData {
            script_bytes,
            datum,
            redeemer: redeemer_hex,
            script_context: String::new(),
            script_version,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_construction() {
        let client = BlockfrostClient::new("test_key".to_string());
        assert_eq!(client.api_key, "test_key");
        assert_eq!(client.base_url, DEFAULT_BASE_URL);
    }

    #[test]
    fn test_client_with_custom_base_url() {
        let client =
            BlockfrostClient::with_base_url("key".to_string(), "https://custom.api.io".to_string());
        assert_eq!(client.base_url, "https://custom.api.io");
    }

    #[test]
    fn test_fetch_with_empty_key() {
        let client = BlockfrostClient::new(String::new());
        let result = client
            .fetch_transaction("0000000000000000000000000000000000000000000000000000000000000000");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("API key is empty"),
            "expected empty key error, got: {err}"
        );
    }

    #[test]
    fn test_fetch_with_invalid_hash() {
        let client = BlockfrostClient::new("some_key".to_string());
        let result = client.fetch_transaction("not_a_valid_hash");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("invalid transaction hash"),
            "expected invalid hash error, got: {err}"
        );
    }

    #[test]
    fn test_from_env_without_var() {
        // Ensure the env var is not set for this test.
        std::env::remove_var(BLOCKFROST_API_KEY_ENV);
        let result = BlockfrostClient::from_env();
        assert!(result.is_err());
    }
}
