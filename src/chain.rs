//! The chain-data rows this recorder acquires, and the one trait that supplies
//! them.
//!
//! # What this module observes
//!
//! Nothing. It holds no network code and reads no file. It defines the row
//! shapes the acquisition path needs — blocks, per-block transaction lists,
//! raw transaction CBOR, scripts, UTxOs, datums and per-epoch protocol
//! parameters — and the JSON decoding that turns a Koios response into them.
//! The decoders are shared by both implementations of [`ChainSource`] on
//! purpose: a live response and a committed fixture of the same endpoint are
//! parsed by the *same* code, so a fixture cannot pass through a lenient path
//! the live one does not take.
//!
//! # What this module does NOT do
//!
//! It does not fetch (see [`crate::koios`]), it does not read fixtures (see
//! [`crate::fixture`]), and it does not decide anything about a transaction.
//! In particular **it does not classify provenance**: whether a script or a
//! datum travels inside a transaction is a reading off the transaction's own
//! CBOR and lives in [`crate::provenance`]. Koios reports a
//! `plutus_contracts[].bytecode` field, and CXP-ADA-2 measured that field to
//! be `null` for transactions whose script bytes ARE present in the
//! transaction, so nothing here may be allowed to stand in for that reading.
//!
//! # Hydration rounds
//!
//! [`ChainSource`] is deliberately shaped as *leaf reads at a stated past
//! point* — `utxos`, `scripts`, `datums` each take a batch of keys — because
//! that is what Chain-Support-Matrix.md §1.4's `hydrated-from-node` row
//! describes, and counting the calls is how a round count becomes a
//! measurement rather than a claim. It is NOT shaped as "ask the node for the
//! minimal touched pre-state", because Cardano serves no such method.

use std::collections::BTreeMap;
use std::fmt;

use eyre::{eyre, Result};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Where a row came from
// ---------------------------------------------------------------------------

/// Whether the rows a source hands back came off the network now or out of a
/// committed capture.
///
/// This is carried through to every report the CLI prints. A measurement whose
/// origin is not stated is a measurement a reader has to trust, which is the
/// thing the whole fixture convention exists to avoid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainOrigin {
    /// Fetched from the live chain during this run.
    Live,
    /// Read from a committed Koios capture under `tests/fixtures/koios/`.
    Fixture,
}

impl fmt::Display for ChainOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChainOrigin::Live => write!(f, "koios-live"),
            ChainOrigin::Fixture => write!(f, "koios-fixture"),
        }
    }
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

/// A row of Koios `/blocks`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockRow {
    pub hash: String,
    pub block_height: u64,
    pub epoch_no: u64,
    /// The protocol version the block's own header announces.
    ///
    /// **This is not the ledger era.** CXP-ADA-2 measured blocks announcing
    /// `proto_major = 5` at height 6,167,631 in epoch 286, three epochs before
    /// the Alonzo era began, and blocks still announcing `4` at height
    /// 6,235,699 in epoch 289. The era is a property of the epoch and is read
    /// from `/epoch_params`; see [`crate::eras`].
    pub proto_major: u64,
    pub proto_minor: u64,
    pub tx_count: u64,
}

/// A row of Koios `/block_txs`: one transaction, named with its block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockTxRow {
    pub block_hash: String,
    pub tx_hash: String,
    pub block_height: u64,
    pub epoch_no: u64,
}

/// A reference to one transaction output.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct UtxoRef {
    pub tx_hash: String,
    pub index: u64,
}

impl UtxoRef {
    pub fn new(tx_hash: impl Into<String>, index: u64) -> Self {
        UtxoRef {
            tx_hash: tx_hash.into(),
            index,
        }
    }

    /// Koios spells a UTxO reference `<tx_hash>#<index>`.
    pub fn to_koios(&self) -> String {
        format!("{}#{}", self.tx_hash, self.index)
    }
}

impl fmt::Display for UtxoRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.tx_hash, self.index)
    }
}

/// A Plutus script as the chain stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptRow {
    pub script_hash: String,
    /// `plutusV1` / `plutusV2` / `plutusV3` / `timelock` / `multisig`.
    pub script_type: String,
    /// The script bytes, or `None` when the source reported none.
    pub bytes: Option<Vec<u8>>,
}

impl ScriptRow {
    /// The one-byte language tag Cardano prepends before hashing a script.
    ///
    /// `None` for a script type that is not a Plutus language, because such a
    /// script is not hashed this way and guessing a tag would manufacture a
    /// hash that matched nothing.
    pub fn language_tag(&self) -> Option<u8> {
        match self.script_type.as_str() {
            "plutusV1" => Some(1),
            "plutusV2" => Some(2),
            "plutusV3" => Some(3),
            _ => None,
        }
    }
}

/// One native asset on an output, from a `/utxo_info` row's `asset_list`.
///
/// Carried because a resolved input's **value** is part of Plutus's
/// `ScriptContext`: a `TxOut` is `(address, value, datum, script_ref)`, so an
/// output reported with its lovelace alone would produce a script context the
/// chain never applied, and the budget computed from it would be a different
/// script's budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetRow {
    /// The minting policy's script hash, hex.
    pub policy_id: String,
    /// The asset name, hex, possibly empty (which is a legal asset name).
    pub asset_name: String,
    pub quantity: u64,
}

/// A row of Koios `/utxo_info` with `_extended: true`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UtxoRow {
    pub reference: UtxoRef,
    /// Whether the output has already been consumed.
    ///
    /// `true` here is the measurement CXP-ADA-2's deliverable 3 rests on: a
    /// spent output is still served, *with* its inline datum, which is what
    /// makes `hydrated-from-node` work at all for a historic transaction.
    pub is_spent: bool,
    pub address: String,
    /// Lovelace on the output.
    pub value_lovelace: u64,
    /// Native assets on the output, in the order the node served them. The
    /// evaluator sorts a value canonically before hashing it into a script
    /// context, so this order is not load-bearing — but the *contents* are.
    pub assets: Vec<AssetRow>,
    pub datum_hash: Option<String>,
    /// The inline datum's bytes (CIP-32), when the output carries one.
    pub inline_datum: Option<Vec<u8>>,
    /// The reference script the output carries (CIP-33), when it carries one.
    pub reference_script: Option<ScriptRow>,
}

/// A row of Koios `/datum_info`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatumRow {
    pub datum_hash: String,
    pub bytes: Vec<u8>,
}

/// A row of the `/epoch_params` projection used to find era boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolVersionRow {
    pub epoch_no: u64,
    pub protocol_major: u64,
    pub protocol_minor: u64,
}

/// One epoch's protocol parameters, as far as era handling needs them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpochParams {
    pub epoch_no: u64,
    /// The ledger era name, or `None` for an epoch that has no parameter row
    /// at all — which is every Byron epoch.
    pub era: Option<String>,
    pub protocol_major: Option<u64>,
    pub protocol_minor: Option<u64>,
    /// Per-language cost-model parameters, in the chain's own index order.
    ///
    /// Koios serves `cost_models` as an object keyed by the parameter's
    /// *numeric index*, so the vector below is positional and is exactly what
    /// `uplc::machine::cost_model::initialize_cost_model` consumes. An epoch
    /// with no Plutus language — every era up to and including Mary — carries
    /// an empty map, and that emptiness is the script-tracing floor.
    pub cost_models: BTreeMap<String, Vec<i64>>,
}

// ---------------------------------------------------------------------------
// The source
// ---------------------------------------------------------------------------

/// The chain reads the acquisition path makes.
///
/// Implemented twice: [`crate::koios::KoiosClient`] against the live network
/// and [`crate::fixture::FixtureSource`] against committed captures. The
/// second exists so the standing tests measure something offline rather than
/// skipping; it is not a mock, because what it returns is the chain's own
/// response bytes and every one of them is checked against a hash the chain
/// itself published (see [`crate::fixture`]).
pub trait ChainSource {
    /// Where these rows came from.
    fn origin(&self) -> ChainOrigin;

    /// Blocks with `lo <= block_height <= hi`, ascending.
    fn blocks_in_range(&self, lo: u64, hi: u64) -> Result<Vec<BlockRow>>;

    /// Every transaction in each of the named blocks.
    fn block_txs(&self, block_hashes: &[String]) -> Result<Vec<BlockTxRow>>;

    /// The raw on-chain CBOR of one transaction.
    fn tx_cbor(&self, tx_hash: &str) -> Result<Vec<u8>>;

    /// Scripts by hash.
    fn scripts(&self, script_hashes: &[String]) -> Result<Vec<ScriptRow>>;

    /// Outputs by reference — the leaf read `hydrated-from-node` is named for.
    fn utxos(&self, refs: &[UtxoRef]) -> Result<Vec<UtxoRow>>;

    /// Datums by hash.
    fn datums(&self, datum_hashes: &[String]) -> Result<Vec<DatumRow>>;

    /// The epoch the transaction's block belongs to.
    ///
    /// This is the one fact that decides which cost model a replay is charged
    /// under, and it cannot be read off the transaction's own bytes: the
    /// post-Alonzo formats are forward-compatible, so a 2021 Alonzo
    /// transaction decodes cleanly as Conway (see [`crate::provenance`]). It
    /// is therefore a chain read rather than a decoder probe — the difference
    /// between charging a transaction under its own era's prices and charging
    /// it under today's.
    fn tx_epoch(&self, tx_hash: &str) -> Result<u64>;

    /// One epoch's protocol parameters.
    fn epoch_params(&self, epoch: u64) -> Result<EpochParams>;

    /// Every epoch's announced protocol version, ascending by epoch.
    fn protocol_versions(&self) -> Result<Vec<ProtocolVersionRow>>;
}

// ---------------------------------------------------------------------------
// Shared JSON decoding
// ---------------------------------------------------------------------------

fn as_array<'a>(v: &'a Value, what: &str) -> Result<&'a Vec<Value>> {
    v.as_array()
        .ok_or_else(|| eyre!("{what}: expected a JSON array, got {}", kind_of(v)))
}

fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn req_u64(v: &Value, field: &str, what: &str) -> Result<u64> {
    v.get(field)
        .and_then(|x| x.as_u64())
        .ok_or_else(|| eyre!("{what}: missing or non-integer field `{field}`"))
}

fn req_str(v: &Value, field: &str, what: &str) -> Result<String> {
    v.get(field)
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| eyre!("{what}: missing or non-string field `{field}`"))
}

/// Read a lovelace or asset quantity.
///
/// Koios serves these as decimal **strings**, because a Cardano quantity does
/// not fit a JSON number's guaranteed integer range; some deployments serve a
/// number. Both are read, and anything else is refused rather than defaulted
/// to zero — a value silently read as zero would build a script context the
/// chain never applied and the budget computed from it would still look like
/// a budget.
fn req_amount(v: &Value, field: &str, what: &str) -> Result<u64> {
    match v.get(field) {
        Some(Value::String(s)) => s
            .parse::<u64>()
            .map_err(|e| eyre!("{what}: field `{field}` is not a u64 quantity (`{s}`): {e}")),
        Some(Value::Number(n)) => n
            .as_u64()
            .ok_or_else(|| eyre!("{what}: field `{field}` is not a non-negative integer")),
        other => Err(eyre!(
            "{what}: missing or non-numeric field `{field}` (got {})",
            other.map(kind_of).unwrap_or("absent")
        )),
    }
}

fn opt_str(v: &Value, field: &str) -> Option<String> {
    v.get(field).and_then(|x| x.as_str()).map(|s| s.to_string())
}

fn opt_hex(v: &Value, field: &str, what: &str) -> Result<Option<Vec<u8>>> {
    match v.get(field).and_then(|x| x.as_str()) {
        None => Ok(None),
        Some(s) => {
            Ok(Some(hex::decode(s).map_err(|e| {
                eyre!("{what}: field `{field}` is not hex: {e}")
            })?))
        }
    }
}

/// Decode a `/blocks` response.
pub fn parse_blocks(v: &Value) -> Result<Vec<BlockRow>> {
    as_array(v, "/blocks")?
        .iter()
        .map(|row| {
            Ok(BlockRow {
                hash: req_str(row, "hash", "/blocks row")?,
                block_height: req_u64(row, "block_height", "/blocks row")?,
                epoch_no: req_u64(row, "epoch_no", "/blocks row")?,
                proto_major: req_u64(row, "proto_major", "/blocks row")?,
                proto_minor: req_u64(row, "proto_minor", "/blocks row")?,
                tx_count: req_u64(row, "tx_count", "/blocks row")?,
            })
        })
        .collect()
}

/// Decode a `/block_txs` response.
pub fn parse_block_txs(v: &Value) -> Result<Vec<BlockTxRow>> {
    as_array(v, "/block_txs")?
        .iter()
        .map(|row| {
            Ok(BlockTxRow {
                block_hash: req_str(row, "block_hash", "/block_txs row")?,
                tx_hash: req_str(row, "tx_hash", "/block_txs row")?,
                block_height: req_u64(row, "block_height", "/block_txs row")?,
                epoch_no: req_u64(row, "epoch_no", "/block_txs row")?,
            })
        })
        .collect()
}

/// Decode a `/tx_cbor` response, returning the bytes for `tx_hash`.
pub fn parse_tx_cbor(v: &Value, tx_hash: &str) -> Result<Vec<u8>> {
    for row in as_array(v, "/tx_cbor")? {
        if row.get("tx_hash").and_then(|x| x.as_str()) == Some(tx_hash) {
            let hexs = req_str(row, "cbor", "/tx_cbor row")?;
            return hex::decode(&hexs)
                .map_err(|e| eyre!("/tx_cbor row for {tx_hash}: cbor is not hex: {e}"));
        }
    }
    Err(eyre!(
        "/tx_cbor carried no row for transaction {tx_hash}; it carried {:?}",
        as_array(v, "/tx_cbor")?
            .iter()
            .filter_map(|r| r.get("tx_hash").and_then(|x| x.as_str()))
            .collect::<Vec<_>>()
    ))
}

/// Decode a script object.
///
/// Koios spells the hash `script_hash` in a `/script_info` row and `hash` in a
/// `/utxo_info` row's embedded `reference_script`. Both are accepted, and a
/// row carrying neither is refused rather than given an empty hash — a script
/// with no identity would compare equal to nothing and make every
/// reference-script resolution read as unresolved.
fn parse_script_row(row: &Value) -> Result<ScriptRow> {
    let script_hash = opt_str(row, "script_hash")
        .or_else(|| opt_str(row, "hash"))
        .ok_or_else(|| eyre!("script row: neither `script_hash` nor `hash` is a string"))?;
    Ok(ScriptRow {
        script_hash,
        script_type: req_str(row, "type", "script row")?,
        bytes: opt_hex(row, "bytes", "script row")?,
    })
}

/// Decode a `/script_info` response.
pub fn parse_scripts(v: &Value) -> Result<Vec<ScriptRow>> {
    as_array(v, "/script_info")?
        .iter()
        .map(parse_script_row)
        .collect()
}

/// Decode a `/utxo_info` response taken with `_extended: true`.
pub fn parse_utxos(v: &Value) -> Result<Vec<UtxoRow>> {
    as_array(v, "/utxo_info")?
        .iter()
        .map(|row| {
            let reference = UtxoRef::new(
                req_str(row, "tx_hash", "/utxo_info row")?,
                req_u64(row, "tx_index", "/utxo_info row")?,
            );
            let inline_datum = match row.get("inline_datum") {
                Some(Value::Object(_)) => opt_hex(&row["inline_datum"], "bytes", "/utxo_info row")?,
                _ => None,
            };
            let reference_script = match row.get("reference_script") {
                Some(Value::Object(_)) => Some(parse_script_row(&row["reference_script"])?),
                _ => None,
            };
            let mut assets: Vec<AssetRow> = Vec::new();
            if let Some(Value::Array(list)) = row.get("asset_list") {
                for a in list {
                    assets.push(AssetRow {
                        policy_id: req_str(a, "policy_id", "/utxo_info asset_list entry")?,
                        // An empty asset name is legal and is served as `""`,
                        // so the field must be present-but-empty rather than
                        // defaulted: a missing field would mean the node said
                        // something this decoder does not understand.
                        asset_name: a
                            .get("asset_name")
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string())
                            .ok_or_else(|| {
                                eyre!("/utxo_info asset_list entry: missing `asset_name`")
                            })?,
                        quantity: req_amount(a, "quantity", "/utxo_info asset_list entry")?,
                    });
                }
            }

            Ok(UtxoRow {
                reference,
                is_spent: row
                    .get("is_spent")
                    .and_then(|x| x.as_bool())
                    .ok_or_else(|| eyre!("/utxo_info row: missing or non-bool `is_spent`"))?,
                address: req_str(row, "address", "/utxo_info row")?,
                value_lovelace: req_amount(row, "value", "/utxo_info row")?,
                assets,
                datum_hash: opt_str(row, "datum_hash"),
                inline_datum,
                reference_script,
            })
        })
        .collect()
}

/// Decode a `/datum_info` response.
pub fn parse_datums(v: &Value) -> Result<Vec<DatumRow>> {
    as_array(v, "/datum_info")?
        .iter()
        .map(|row| {
            Ok(DatumRow {
                datum_hash: req_str(row, "datum_hash", "/datum_info row")?,
                bytes: opt_hex(row, "bytes", "/datum_info row")?
                    .ok_or_else(|| eyre!("/datum_info row: missing `bytes`"))?,
            })
        })
        .collect()
}

/// Decode an `/epoch_params` response for one epoch.
///
/// An empty array is not an error: Byron epochs have no parameter row, and
/// that absence is itself a measurement. It is returned as an `EpochParams`
/// with `era: None` and no cost models.
pub fn parse_epoch_params(v: &Value, epoch: u64) -> Result<EpochParams> {
    let rows = as_array(v, "/epoch_params")?;
    let Some(row) = rows.first() else {
        return Ok(EpochParams {
            epoch_no: epoch,
            era: None,
            protocol_major: None,
            protocol_minor: None,
            cost_models: BTreeMap::new(),
        });
    };

    let mut cost_models = BTreeMap::new();
    if let Some(Value::Object(langs)) = row.get("cost_models") {
        for (lang, params) in langs {
            // Koios serves a cost model POSITIONALLY. Measured 2026-10-31:
            // mainnet returns a JSON *array* of integers per language. Some
            // deployments and some older captures return an object keyed by
            // the parameter's numeric index instead, so both are accepted and
            // both are reduced to the same positional vector — which is what
            // `initialize_cost_model` consumes.
            //
            // Either way the vector must be DENSE from 0: that function
            // indexes its slice positionally, so a hole would silently shift
            // every later parameter and charge the wrong price.
            let indexed: Vec<(usize, i64)> = match params {
                Value::Array(items) => {
                    let mut out = Vec::with_capacity(items.len());
                    for (i, val) in items.iter().enumerate() {
                        let n = val.as_i64().ok_or_else(|| {
                            eyre!(
                                "/epoch_params epoch {epoch}: cost_models.{lang}[{i}] is {}, \
                                 expected an integer",
                                kind_of(val)
                            )
                        })?;
                        out.push((i, n));
                    }
                    out
                }
                Value::Object(by_index) => {
                    let mut out: Vec<(usize, i64)> = Vec::with_capacity(by_index.len());
                    for (k, val) in by_index {
                        let idx: usize = k.parse().map_err(|_| {
                            eyre!(
                                "/epoch_params epoch {epoch}: cost_models.{lang} has non-numeric \
                                 key `{k}`, so its parameters cannot be put in the chain's own \
                                 order"
                            )
                        })?;
                        let n = val.as_i64().ok_or_else(|| {
                            eyre!(
                                "/epoch_params epoch {epoch}: cost_models.{lang}[{k}] is {}, \
                                 expected an integer",
                                kind_of(val)
                            )
                        })?;
                        out.push((idx, n));
                    }
                    out
                }
                other => {
                    return Err(eyre!(
                        "/epoch_params epoch {epoch}: cost_models.{lang} is {}, expected an array \
                         of parameters or an object keyed by parameter index",
                        kind_of(other)
                    ))
                }
            };
            let mut indexed = indexed;
            indexed.sort_unstable_by_key(|(i, _)| *i);
            for (expected, (got, _)) in indexed.iter().enumerate() {
                if expected != *got {
                    return Err(eyre!(
                        "/epoch_params epoch {epoch}: cost_models.{lang} is not a dense 0..n-1 \
                         index map — index {expected} is missing (next present is {got}), so a \
                         positional cost model built from it would charge the wrong parameter"
                    ));
                }
            }
            cost_models.insert(lang.clone(), indexed.into_iter().map(|(_, n)| n).collect());
        }
    }

    Ok(EpochParams {
        epoch_no: row
            .get("epoch_no")
            .and_then(|x| x.as_u64())
            .unwrap_or(epoch),
        era: opt_str(row, "era"),
        protocol_major: row.get("protocol_major").and_then(|x| x.as_u64()),
        protocol_minor: row.get("protocol_minor").and_then(|x| x.as_u64()),
        cost_models,
    })
}

/// Decode a `/tx_info` projection, returning the epoch for `tx_hash`.
pub fn parse_tx_epoch(v: &Value, tx_hash: &str) -> Result<u64> {
    for row in as_array(v, "/tx_info projection")? {
        if row.get("tx_hash").and_then(|x| x.as_str()) == Some(tx_hash) {
            return req_u64(row, "epoch_no", "/tx_info projection row");
        }
    }
    Err(eyre!(
        "no /tx_info row carries the epoch of transaction {tx_hash}; the rows carried {:?}",
        as_array(v, "/tx_info projection")?
            .iter()
            .filter_map(|r| r.get("tx_hash").and_then(|x| x.as_str()))
            .collect::<Vec<_>>()
    ))
}

/// Decode the `epoch_no,protocol_major,protocol_minor` projection.
pub fn parse_protocol_versions(v: &Value) -> Result<Vec<ProtocolVersionRow>> {
    let mut out: Vec<ProtocolVersionRow> = as_array(v, "/epoch_params projection")?
        .iter()
        .map(|row| {
            Ok(ProtocolVersionRow {
                epoch_no: req_u64(row, "epoch_no", "protocol version row")?,
                protocol_major: req_u64(row, "protocol_major", "protocol version row")?,
                protocol_minor: req_u64(row, "protocol_minor", "protocol version row")?,
            })
        })
        .collect::<Result<_>>()?;
    out.sort_unstable_by_key(|r| r.epoch_no);
    Ok(out)
}

// ---------------------------------------------------------------------------
// Locating a transaction's epoch
// ---------------------------------------------------------------------------

/// Find the epoch a transaction's block belongs to, by enumerating a window.
///
/// Returns `None` when no block in `[lo, hi]` carries the transaction. That is
/// the honest answer for a transaction outside the window, and it is why
/// callers must report an unresolved era as unknown rather than taking the
/// decoder's probe for a ledger era — a bare transaction's bytes do not say
/// which era they were settled in.
pub fn epoch_of_tx(src: &dyn ChainSource, tx_hash: &str, lo: u64, hi: u64) -> Result<Option<u64>> {
    let blocks = src.blocks_in_range(lo, hi)?;
    let hashes: Vec<String> = blocks.iter().map(|b| b.hash.clone()).collect();
    Ok(src
        .block_txs(&hashes)?
        .into_iter()
        .find(|r| r.tx_hash == tx_hash)
        .map(|r| r.epoch_no))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn utxo_ref_spells_koios_form() {
        assert_eq!(UtxoRef::new("abcd", 3).to_koios(), "abcd#3");
    }

    #[test]
    fn language_tag_refuses_a_non_plutus_script() {
        let native = ScriptRow {
            script_hash: "x".into(),
            script_type: "timelock".into(),
            bytes: None,
        };
        assert_eq!(native.language_tag(), None);
    }

    #[test]
    fn cost_models_are_accepted_in_both_shapes_koios_serves() {
        // Mainnet serves an array; the object-keyed-by-index form is also
        // accepted. Both must reduce to the same positional vector, because
        // the evaluator indexes it positionally.
        let arr = json!([{ "epoch_no": 290, "era": "Alonzo", "cost_models": { "PlutusV1": [10, 20, 30] } }]);
        let obj = json!([{ "epoch_no": 290, "era": "Alonzo", "cost_models": { "PlutusV1": { "0": 10, "1": 20, "2": 30 } } }]);
        let a = parse_epoch_params(&arr, 290).unwrap();
        let o = parse_epoch_params(&obj, 290).unwrap();
        assert_eq!(a.cost_models["PlutusV1"], vec![10i64, 20, 30]);
        assert_eq!(a.cost_models, o.cost_models);
    }

    #[test]
    fn a_cost_model_of_the_wrong_json_type_is_refused() {
        let v = json!([{ "epoch_no": 290, "cost_models": { "PlutusV1": "166 params" } }]);
        let err = parse_epoch_params(&v, 290).expect_err("a string is not a parameter vector");
        assert!(err.to_string().contains("expected an array"), "got: {err}");
    }

    #[test]
    fn cost_models_with_a_hole_are_refused() {
        // Index 1 missing: a positional vector built from this would put
        // parameter 2's value where parameter 1's belongs.
        let v = json!([{ "epoch_no": 290, "era": "Alonzo", "cost_models": { "PlutusV1": { "0": 10, "2": 30 } } }]);
        let err = parse_epoch_params(&v, 290).expect_err("a hole must be refused");
        let msg = err.to_string();
        assert!(
            msg.contains("dense") && msg.contains("index 1 is missing"),
            "refusal must name the missing index; got: {msg}"
        );
    }

    #[test]
    fn byron_epoch_with_no_parameter_row_is_not_an_error() {
        let p = parse_epoch_params(&json!([]), 0).expect("an empty row set is a reading");
        assert_eq!(p.epoch_no, 0);
        assert_eq!(p.era, None);
        assert!(p.cost_models.is_empty());
    }

    #[test]
    fn tx_cbor_for_an_absent_hash_names_what_was_present() {
        let v = json!([{ "tx_hash": "aa", "cbor": "00" }]);
        let err = parse_tx_cbor(&v, "bb").expect_err("an absent hash must be refused");
        assert!(
            err.to_string().contains("\"aa\""),
            "refusal must name the rows it did carry; got: {err}"
        );
    }
}
