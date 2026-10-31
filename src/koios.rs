//! The keyless live chain client.
//!
//! # What this module observes
//!
//! Cardano mainnet, through the public Koios API, with **no API key**. It is
//! the only network code in this crate that can reach the chain for the
//! acquisition path, and it exists because the alternative could not: Koios
//! needs no credential, while `crate::blockfrost` needs one that no CI lane
//! and no developer checkout in this workspace has.
//!
//! Every method is one HTTP call, and the count of calls is the thing
//! Chain-Support-Matrix.md §1.4's `hydrated-from-node` cost column is about
//! (*"O(leaves touched), several RPC calls per transaction"*). Nothing here
//! batches behind the caller's back or caches, so a round count a caller
//! reports is the round count that happened.
//!
//! # What this module does NOT do
//!
//! - It does not interpret a transaction. `tx_cbor` hands back raw bytes and
//!   nothing else; the reading of those bytes is [`crate::provenance`]'s.
//! - It does not use Koios's own `plutus_contracts` summary at all. That
//!   summary reports `bytecode: null` for transactions whose script bytes are
//!   measurably present in the transaction CBOR (CXP-ADA-2, 2026-10-31), so
//!   treating it as a statement about what the transaction carries would
//!   produce exactly the wrong answer about self-containment.
//! - It does not retry, rate-limit or back off. A failed call is an error with
//!   the endpoint in it rather than a silent empty result.

use eyre::{eyre, Context, Result};
use serde_json::{json, Value};

use crate::chain::{
    parse_block_txs, parse_blocks, parse_datums, parse_epoch_params, parse_protocol_versions,
    parse_scripts, parse_tx_cbor, parse_utxos, BlockRow, BlockTxRow, ChainOrigin, ChainSource,
    DatumRow, EpochParams, ProtocolVersionRow, ScriptRow, UtxoRef, UtxoRow,
};

/// Public Koios mainnet base URL. No credential is required or sent.
pub const DEFAULT_BASE_URL: &str = "https://api.koios.rest/api/v1";

/// Environment variable that opts a run in to reaching the live chain.
///
/// Tests consult this and, when it is unset, report **NOT MEASURED** loudly
/// rather than passing or skipping — the convention
/// `tests/verify-cli-convention-no-silent-skip.sh` enforces.
pub const LIVE_CHAIN_ENV: &str = "CODETRACER_CARDANO_RECORDER_LIVE_CHAIN";

/// Whether this run is allowed to reach the live chain.
pub fn live_chain_enabled() -> bool {
    match std::env::var(LIVE_CHAIN_ENV) {
        Ok(v) => {
            let v = v.trim();
            v == "1" || v.eq_ignore_ascii_case("true")
        }
        Err(_) => false,
    }
}

/// A keyless Koios client.
#[derive(Debug, Clone)]
pub struct KoiosClient {
    pub base_url: String,
    /// Number of HTTP calls this client has made, so a caller can report a
    /// hydration round count it measured rather than one it assumed.
    calls: std::cell::Cell<u32>,
}

impl Default for KoiosClient {
    fn default() -> Self {
        Self::new()
    }
}

impl KoiosClient {
    pub fn new() -> Self {
        KoiosClient {
            base_url: DEFAULT_BASE_URL.to_string(),
            calls: std::cell::Cell::new(0),
        }
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        KoiosClient {
            base_url: base_url.into(),
            calls: std::cell::Cell::new(0),
        }
    }

    /// How many HTTP calls this client has made.
    pub fn call_count(&self) -> u32 {
        self.calls.get()
    }

    fn get(&self, path_and_query: &str) -> Result<Value> {
        let url = format!("{}/{}", self.base_url, path_and_query);
        self.calls.set(self.calls.get() + 1);
        let mut resp = ureq::get(&url)
            .call()
            .with_context(|| format!("GET {url} failed"))?;
        resp.body_mut()
            .read_json()
            .with_context(|| format!("GET {url} returned a body that is not JSON"))
    }

    fn post(&self, path: &str, body: Value) -> Result<Value> {
        let url = format!("{}/{}", self.base_url, path);
        self.calls.set(self.calls.get() + 1);
        let mut resp = ureq::post(&url)
            .header("content-type", "application/json")
            .send_json(&body)
            .with_context(|| format!("POST {url} failed"))?;
        resp.body_mut()
            .read_json()
            .with_context(|| format!("POST {url} returned a body that is not JSON"))
    }
}

impl ChainSource for KoiosClient {
    fn origin(&self) -> ChainOrigin {
        ChainOrigin::Live
    }

    fn blocks_in_range(&self, lo: u64, hi: u64) -> Result<Vec<BlockRow>> {
        if hi < lo {
            return Err(eyre!(
                "block range is inverted: from {lo} to {hi}; a range that cannot contain a block \
                 would enumerate zero and report it as a measurement"
            ));
        }
        let v = self.get(&format!(
            "blocks?block_height=gte.{lo}&block_height=lte.{hi}&order=block_height.asc"
        ))?;
        parse_blocks(&v)
    }

    fn block_txs(&self, block_hashes: &[String]) -> Result<Vec<BlockTxRow>> {
        if block_hashes.is_empty() {
            return Ok(Vec::new());
        }
        let v = self.post("block_txs", json!({ "_block_hashes": block_hashes }))?;
        parse_block_txs(&v)
    }

    fn tx_cbor(&self, tx_hash: &str) -> Result<Vec<u8>> {
        let v = self.post("tx_cbor", json!({ "_tx_hashes": [tx_hash] }))?;
        parse_tx_cbor(&v, tx_hash)
    }

    fn scripts(&self, script_hashes: &[String]) -> Result<Vec<ScriptRow>> {
        if script_hashes.is_empty() {
            return Ok(Vec::new());
        }
        let v = self.post("script_info", json!({ "_script_hashes": script_hashes }))?;
        parse_scripts(&v)
    }

    fn utxos(&self, refs: &[UtxoRef]) -> Result<Vec<UtxoRow>> {
        if refs.is_empty() {
            return Ok(Vec::new());
        }
        let keys: Vec<String> = refs.iter().map(|r| r.to_koios()).collect();
        let v = self.post(
            "utxo_info",
            json!({ "_utxo_refs": keys, "_extended": true }),
        )?;
        parse_utxos(&v)
    }

    fn datums(&self, datum_hashes: &[String]) -> Result<Vec<DatumRow>> {
        if datum_hashes.is_empty() {
            return Ok(Vec::new());
        }
        let v = self.post("datum_info", json!({ "_datum_hashes": datum_hashes }))?;
        parse_datums(&v)
    }

    fn epoch_params(&self, epoch: u64) -> Result<EpochParams> {
        let v = self.get(&format!("epoch_params?_epoch_no={epoch}"))?;
        parse_epoch_params(&v, epoch)
    }

    fn protocol_versions(&self) -> Result<Vec<ProtocolVersionRow>> {
        let v = self.get(
            "epoch_params?select=epoch_no,protocol_major,protocol_minor\
             &order=epoch_no.asc&limit=1000",
        )?;
        parse_protocol_versions(&v)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inverted_range_is_refused_before_any_call() {
        let c = KoiosClient::new();
        let err = c
            .blocks_in_range(100, 99)
            .expect_err("an inverted range must be refused");
        assert!(err.to_string().contains("inverted"), "got: {err}");
        assert_eq!(
            c.call_count(),
            0,
            "the refusal must happen before the network is touched"
        );
    }

    #[test]
    fn empty_batches_make_no_call() {
        let c = KoiosClient::new();
        assert!(c.block_txs(&[]).unwrap().is_empty());
        assert!(c.scripts(&[]).unwrap().is_empty());
        assert!(c.utxos(&[]).unwrap().is_empty());
        assert!(c.datums(&[]).unwrap().is_empty());
        assert_eq!(c.call_count(), 0);
    }

    #[test]
    fn the_client_carries_no_credential_field() {
        // Keylessness is the premise of this whole acquisition path, so it is
        // asserted rather than described: the debug rendering of the client
        // must mention no key, token or project id.
        let rendered = format!("{:?}", KoiosClient::new()).to_lowercase();
        for forbidden in ["api_key", "project_id", "token", "secret"] {
            assert!(
                !rendered.contains(forbidden),
                "the keyless client must carry no `{forbidden}`; got: {rendered}"
            );
        }
    }
}
