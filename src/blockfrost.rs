//! Blockfrost API client for reading Cardano transaction data.
//!
//! # What this module observes
//!
//! The [Blockfrost](https://blockfrost.io/) REST API, with an API key: a
//! transaction's UTxOs, the script bytes behind a script hash, the script's
//! Plutus version, and the execution units the chain recorded per redeemer.
//!
//! # What this module does NOT do, and this is the load-bearing part
//!
//! **It cannot assemble anything that can be executed, and it no longer
//! pretends to.** Two things are missing and neither is a tooling gap:
//!
//! - `/txs/{hash}/redeemers` serves `redeemer_data_hash` — the hash of the
//!   redeemer, not the redeemer. Applying a script to a hash is applying it to
//!   a different argument.
//! - No endpoint here yields a `ScriptContext`. One is assembled from the
//!   transaction body plus every resolved input, which is [`crate::oracle`]'s
//!   job over a [`crate::chain::ChainSource`].
//!
//! So this module reports what it read and stops. The previous
//! `fetch_transaction` returned a `TransactionData` with `script_context:
//! String::new()` and the data hash in the redeemer field, which produced a
//! program that ran and whose budget could not match the chain's for reasons
//! unrelated to stepping. That path is removed.
//!
//! **No API key exists in any lane of this workspace** — not in CI and not in
//! a developer checkout — so nothing standing is measured through here.
//! [`crate::koios`] is the keyless route and is what the acquisition path and
//! the budget oracle use.

use eyre::{eyre, Result};
use uplc::machine::cost_model::ExBudget;

use crate::transaction::PlutusVersion;

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

    /// Read what this client *can* read about a transaction's scripts, and say
    /// so.
    ///
    /// Returns a human-readable report rather than a `TransactionData`,
    /// because a `TransactionData` is a thing that can be *executed* and this
    /// client cannot assemble one:
    ///
    /// - `/txs/{hash}/redeemers` serves `redeemer_data_hash` and **not** the
    ///   redeemer data. A script applied to a hash is a different script
    ///   application.
    /// - No endpoint here yields a `ScriptContext`. Assembling one needs the
    ///   transaction body plus every resolved input, which is what
    ///   [`crate::oracle`] does from [`crate::chain::ChainSource`].
    ///
    /// It used to return a `TransactionData` with `script_context:
    /// String::new()` and the data hash in the redeemer field — a placeholder
    /// that produced a runnable program whose budget could not match the
    /// chain's for reasons unrelated to stepping. That is removed rather than
    /// improved: the honest shape for this client is a report of what it read.
    ///
    /// The execution units it reads ARE the chain's own figures, so they are
    /// reported; they are just not comparable to anything this client can
    /// execute.
    pub fn fetch_script_summary(&self, tx_hash: &str) -> Result<String> {
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

        let mut out = format!(
            "transaction {tx_hash} (via Blockfrost)\n  script {script_hash} ({script_version}), \
             {} byte(s)\n  {} redeemer(s)\n",
            script_bytes.len(),
            redeemers.len()
        );
        for (i, entry) in redeemers.iter().enumerate() {
            match parse_execution_units(entry) {
                Some(units) => out.push_str(&format!(
                    "    redeemer[{i}] chain-recorded mem={} steps={}\n",
                    units.mem, units.cpu
                )),
                None => out.push_str(&format!(
                    "    redeemer[{i}] chain-recorded units NOT REPORTED by this endpoint\n"
                )),
            }
            out.push_str(&format!(
                "    redeemer[{i}] data: NOT AVAILABLE -- this endpoint serves \
                 redeemer_data_hash={}, which is the hash and not the data\n",
                entry["redeemer_data_hash"].as_str().unwrap_or("?")
            ));
        }
        out.push_str(
            "  script context: NOT AVAILABLE -- no endpoint here yields one, and assembling one \
             needs the body plus every resolved input. So nothing here can be stepped and no \
             budget comparison is run from this path.\n",
        );
        Ok(out)
    }
}

/// Extract the consensus-recorded execution units from a Blockfrost
/// `/txs/{hash}/redeemers` entry.
///
/// Blockfrost encodes `unit_mem` and `unit_steps` as decimal strings; some
/// deployments have been observed to return them as JSON numbers, so both are
/// accepted.  Returns `None` unless *both* axes are present and parseable: a
/// half-known budget is not an oracle, and silently defaulting the missing
/// axis to zero would manufacture a divergence (or an agreement) that the
/// chain never reported.
fn parse_execution_units(entry: &serde_json::Value) -> Option<ExBudget> {
    let field = |name: &str| -> Option<i64> {
        let v = entry.get(name)?;
        match v {
            serde_json::Value::String(s) => s.parse::<i64>().ok(),
            serde_json::Value::Number(n) => n.as_i64(),
            _ => None,
        }
    };

    Some(ExBudget {
        mem: field("unit_mem")?,
        cpu: field("unit_steps")?,
    })
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
        let result = client.fetch_script_summary(
            "0000000000000000000000000000000000000000000000000000000000000000",
        );
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
        let result = client.fetch_script_summary("not_a_valid_hash");
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
