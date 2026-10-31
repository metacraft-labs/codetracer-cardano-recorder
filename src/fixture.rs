//! Committed Koios captures, and the hash identities that make them readable
//! rather than trusted.
//!
//! # What this module observes
//!
//! Text files under `tests/fixtures/koios/`, each an envelope carrying one
//! endpoint's response **verbatim** together with the endpoint, the query and
//! the date it was fetched. It exists so the acquisition suite measures
//! something with no network and no credential — CI has neither — instead of
//! skipping.
//!
//! # Why this is not a mock
//!
//! A mock returns what its author decided. These files return what Cardano
//! mainnet returned, and the difference is **checkable inside the file**:
//!
//! | identity | the reading |
//! |---|---|
//! | `datum-blake2b-256` | `blake2b-256(inline_datum.bytes) == datum_hash` |
//! | `script-blake2b-224-tagged` | `blake2b-224(language_tag ‖ script_bytes) == script_hash` |
//! | `tx-body-blake2b-256` | `blake2b-256(transaction BODY's original CBOR) == tx_hash`, and `blake2b-256(whole transaction CBOR) != tx_hash` |
//! | `cross-fixture` | two captures of different endpoints must agree about the same blocks |
//! | `evaluator-arity` | each cost model is a dense `0..n-1` index map, and its length is compared against the arity the evaluator reads |
//! | `none` | *stated as unanchored* — see below |
//!
//! [`verify_identities`] runs all of them and returns one reading per
//! fixture, so "these are the chain's bytes" is something a test asserts
//! rather than something a reader takes on faith about whoever ran the fetch.
//!
//! **Two fixtures are honestly unanchored and say so.** A protocol-parameter
//! set and a projection of epoch/protocol-version pairs are not hashed by the
//! chain in any form a client can recompute, so their `identity.kind` is
//! `none` or `evaluator-arity` and the verifier reports them as such instead
//! of inventing an anchor. An unanchored fixture is still a capture with a
//! date and an endpoint on it; it is just not self-proving, and a reader
//! should know which is which.
//!
//! # What this module does NOT do
//!
//! It does not fetch, refresh or write a fixture: regenerating one is a
//! deliberate act with a dated provenance line, not a side effect of a test
//! run. It does not fall back to the network when a fixture is missing — a
//! missing fixture is an error naming the file, because a silent fallback
//! would make an offline suite pass for the wrong reason.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use eyre::{eyre, Context, Result};
use pallas_crypto::hash::Hasher;
use serde_json::Value;

use crate::chain::{
    parse_block_txs, parse_blocks, parse_datums, parse_epoch_params, parse_protocol_versions,
    parse_scripts, parse_tx_cbor, parse_utxos, BlockRow, BlockTxRow, ChainOrigin, ChainSource,
    DatumRow, EpochParams, ProtocolVersionRow, ScriptRow, UtxoRef, UtxoRow,
};

/// The three transaction cases the fixtures carry, each chosen for what it
/// measures rather than for being typical.
pub mod cases {
    /// The earliest Plutus script execution reachable on mainnet — Alonzo,
    /// PlutusV1, block 6,236,063. Its script and its spent output's datum are
    /// both carried in the transaction, which is the `self-contained` era's
    /// positive arm.
    pub const FIRST_PLUTUS_EXECUTION: &str = "first_plutus_execution";
    /// A live PlutusV3 spend whose scripts are CIP-33 reference scripts and
    /// whose spent outputs' datums are CIP-32 inline datums: neither travels
    /// with the transaction.
    pub const V3_REFERENCE_SCRIPT_SPEND: &str = "v3_reference_script_spend";
    /// A live PlutusV3 spend whose script IS carried in the transaction while
    /// the spent output's inline datum is not — the case that stops the
    /// post-Babbage reading from being "the script is always absent".
    pub const V3_EMBEDDED_SCRIPT_SPEND: &str = "v3_embedded_script_spend";

    /// Every case, in the order a report should print them.
    pub const ALL: [&str; 3] = [
        FIRST_PLUTUS_EXECUTION,
        V3_REFERENCE_SCRIPT_SPEND,
        V3_EMBEDDED_SCRIPT_SPEND,
    ];
}

/// The bounded historic range the enumeration fixtures cover: the first ten
/// blocks of the Alonzo era.
pub const ALONZO_WINDOW: (u64, u64) = (6_236_060, 6_236_069);

/// The epochs the era-boundary fixture carries parameters for.
pub const BOUNDARY_EPOCHS: [u64; 13] = [
    0, 207, 208, 236, 251, 290, 298, 365, 394, 507, 537, 644, 664,
];

// ---------------------------------------------------------------------------
// The envelope
// ---------------------------------------------------------------------------

/// One committed capture.
#[derive(Debug, Clone)]
pub struct Fixture {
    /// File name, without the directory.
    pub name: String,
    pub network: String,
    pub endpoint: String,
    pub query: Value,
    /// ISO-8601 date the capture was taken.
    pub fetched: String,
    /// What makes this capture self-checkable, verbatim from the file.
    pub identity_kind: String,
    pub identity_note: String,
    pub identity: Value,
    /// The endpoint's response, verbatim.
    pub response: Value,
}

impl Fixture {
    fn from_json(name: &str, v: Value) -> Result<Self> {
        let s = |k: &str| -> Result<String> {
            v.get(k)
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| eyre!("fixture {name}: missing or non-string `{k}`"))
        };
        let identity = v
            .get("identity")
            .cloned()
            .ok_or_else(|| eyre!("fixture {name}: missing `identity`"))?;
        Ok(Fixture {
            name: name.to_string(),
            network: s("network")?,
            endpoint: s("endpoint")?,
            fetched: s("fetched")?,
            query: v.get("query").cloned().unwrap_or(Value::Null),
            identity_kind: identity
                .get("kind")
                .and_then(|x| x.as_str())
                .ok_or_else(|| eyre!("fixture {name}: `identity.kind` must be a string"))?
                .to_string(),
            identity_note: identity
                .get("note")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            identity,
            response: v
                .get("response")
                .cloned()
                .ok_or_else(|| eyre!("fixture {name}: missing `response`"))?,
        })
    }
}

// ---------------------------------------------------------------------------
// The source
// ---------------------------------------------------------------------------

/// A [`ChainSource`] backed by the committed captures.
#[derive(Debug, Clone)]
pub struct FixtureSource {
    dir: PathBuf,
}

/// The fixture directory relative to the crate root.
pub const FIXTURE_DIR: &str = "tests/fixtures/koios";

impl Default for FixtureSource {
    fn default() -> Self {
        Self::at_crate_root()
    }
}

impl FixtureSource {
    /// Open the fixture directory under this crate's source root.
    ///
    /// `CARGO_MANIFEST_DIR` is used rather than the process working directory
    /// because a test binary's working directory is the crate root while an
    /// installed binary's is wherever the user stood, and a report that
    /// silently found no fixtures would be the wrong kind of quiet.
    pub fn at_crate_root() -> Self {
        FixtureSource {
            dir: Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_DIR),
        }
    }

    pub fn at(dir: impl Into<PathBuf>) -> Self {
        FixtureSource { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Load one capture by its stem (without `.fixture.json`).
    pub fn load(&self, stem: &str) -> Result<Fixture> {
        let path = self.dir.join(format!("{stem}.fixture.json"));
        let text = std::fs::read_to_string(&path).with_context(|| {
            format!(
                "fixture `{stem}` is not present at {}. Fixtures are committed captures; a \
                 missing one is not a reason to reach the network, because an offline suite that \
                 silently went online would be measuring a different thing on every machine",
                path.display()
            )
        })?;
        let v: Value = serde_json::from_str(&text)
            .with_context(|| format!("fixture {} is not valid JSON", path.display()))?;
        Fixture::from_json(stem, v)
    }

    /// Every capture in the directory, by stem, in sorted order.
    pub fn stems(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&self.dir)
            .with_context(|| format!("cannot list fixtures at {}", self.dir.display()))?
        {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(stem) = name.strip_suffix(".fixture.json") {
                out.push(stem.to_string());
            }
        }
        out.sort();
        Ok(out)
    }
}

impl ChainSource for FixtureSource {
    fn origin(&self) -> ChainOrigin {
        ChainOrigin::Fixture
    }

    fn blocks_in_range(&self, lo: u64, hi: u64) -> Result<Vec<BlockRow>> {
        let f = self.load("blocks_alonzo_window")?;
        let all = parse_blocks(&f.response)?;
        let kept: Vec<BlockRow> = all
            .iter()
            .filter(|b| b.block_height >= lo && b.block_height <= hi)
            .cloned()
            .collect();
        if kept.is_empty() {
            return Err(eyre!(
                "no committed block capture covers heights {lo}..={hi}. The captured window is \
                 {}..={} (`{}`); a range outside it is NOT MEASURED offline and needs \
                 --live",
                ALONZO_WINDOW.0,
                ALONZO_WINDOW.1,
                f.name
            ));
        }
        Ok(kept)
    }

    fn block_txs(&self, block_hashes: &[String]) -> Result<Vec<BlockTxRow>> {
        let f = self.load("block_txs_alonzo_window")?;
        let all = parse_block_txs(&f.response)?;
        let missing: Vec<&String> = block_hashes
            .iter()
            .filter(|h| !all.iter().any(|r| &r.block_hash == *h))
            .collect();
        if !missing.is_empty() {
            return Err(eyre!(
                "committed capture `{}` carries no transaction rows for {} of the {} requested \
                 blocks (first missing: {}). A block present in one capture and absent from the \
                 other would make an enumerated count silently short",
                f.name,
                missing.len(),
                block_hashes.len(),
                missing[0]
            ));
        }
        Ok(all
            .into_iter()
            .filter(|r| block_hashes.contains(&r.block_hash))
            .collect())
    }

    fn tx_cbor(&self, tx_hash: &str) -> Result<Vec<u8>> {
        for case in cases::ALL {
            let f = self.load(&format!("tx_cbor_{case}"))?;
            if let Ok(bytes) = parse_tx_cbor(&f.response, tx_hash) {
                return Ok(bytes);
            }
        }
        Err(eyre!(
            "no committed capture carries the CBOR of transaction {tx_hash}. The cases captured \
             are {:?}; another transaction is NOT MEASURED offline and needs --live",
            cases::ALL
        ))
    }

    fn scripts(&self, script_hashes: &[String]) -> Result<Vec<ScriptRow>> {
        let mut found: Vec<ScriptRow> = Vec::new();
        for case in cases::ALL {
            let f = self.load(&format!("script_info_{case}"))?;
            for row in parse_scripts(&f.response)? {
                if script_hashes.contains(&row.script_hash)
                    && !found.iter().any(|r| r.script_hash == row.script_hash)
                {
                    found.push(row);
                }
            }
        }
        Ok(found)
    }

    fn utxos(&self, refs: &[UtxoRef]) -> Result<Vec<UtxoRow>> {
        let mut found: Vec<UtxoRow> = Vec::new();
        for case in cases::ALL {
            let f = self.load(&format!("utxo_info_{case}"))?;
            for row in parse_utxos(&f.response)? {
                if refs.contains(&row.reference)
                    && !found.iter().any(|r| r.reference == row.reference)
                {
                    found.push(row);
                }
            }
        }
        Ok(found)
    }

    fn datums(&self, datum_hashes: &[String]) -> Result<Vec<DatumRow>> {
        let f = self.load("datum_info_first_plutus_execution")?;
        Ok(parse_datums(&f.response)?
            .into_iter()
            .filter(|d| datum_hashes.contains(&d.datum_hash))
            .collect())
    }

    fn epoch_params(&self, epoch: u64) -> Result<EpochParams> {
        let f = self.load("epoch_params_era_boundaries")?;
        let per_epoch = f.response.as_object().ok_or_else(|| {
            eyre!(
                "fixture {}: `response` must be an object keyed by epoch",
                f.name
            )
        })?;
        let raw = per_epoch.get(&epoch.to_string()).ok_or_else(|| {
            eyre!(
                "no committed parameter capture for epoch {epoch}. The epochs captured are \
                 {:?}; another epoch is NOT MEASURED offline and needs --live",
                BOUNDARY_EPOCHS
            )
        })?;
        parse_epoch_params(raw, epoch)
    }

    fn protocol_versions(&self) -> Result<Vec<ProtocolVersionRow>> {
        let f = self.load("protocol_versions_by_epoch")?;
        parse_protocol_versions(&f.response)
    }
}

// ---------------------------------------------------------------------------
// Hash identities
// ---------------------------------------------------------------------------

/// Whether a fixture's internal identity held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityVerdict {
    /// Every identity the fixture declares was recomputed and agreed.
    Anchored,
    /// The fixture declares no recomputable identity. This is a statement,
    /// not a pass: see the module docs.
    Unanchored,
    /// An identity was recomputed and disagreed.
    Broken,
}

impl fmt::Display for IdentityVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdentityVerdict::Anchored => write!(f, "ANCHORED"),
            IdentityVerdict::Unanchored => write!(f, "UNANCHORED (stated, not proven)"),
            IdentityVerdict::Broken => write!(f, "BROKEN"),
        }
    }
}

/// One fixture's identity reading.
#[derive(Debug, Clone)]
pub struct IdentityReading {
    pub fixture: String,
    pub kind: String,
    pub verdict: IdentityVerdict,
    /// How many individual hashes (or cross-checks) were recomputed.
    pub checks: usize,
    /// Every disagreement, named.
    pub failures: Vec<String>,
    pub endpoint: String,
    pub fetched: String,
}

fn blake2b_256(bytes: &[u8]) -> String {
    Hasher::<256>::hash(bytes).to_string()
}

fn blake2b_224_tagged(tag: u8, bytes: &[u8]) -> String {
    Hasher::<224>::hash_tagged(bytes, tag).to_string()
}

/// Recompute every identity every committed fixture declares.
///
/// Returns one reading per fixture. A `Broken` verdict is the one that matters:
/// it says the bytes in the file are not the bytes the chain hashed, which is
/// the only failure mode a committed capture has that a date and an endpoint
/// cannot catch.
pub fn verify_identities(src: &FixtureSource) -> Result<Vec<IdentityReading>> {
    let mut out = Vec::new();
    for stem in src.stems()? {
        let f = src.load(&stem)?;
        let mut checks = 0usize;
        let mut failures: Vec<String> = Vec::new();

        match f.identity_kind.as_str() {
            "datum-blake2b-256" => {
                // Two shapes carry a datum: /utxo_info rows (inline_datum) and
                // /datum_info rows (bytes).
                if let Ok(rows) = parse_utxos(&f.response) {
                    for row in rows {
                        if let (Some(bytes), Some(want)) = (&row.inline_datum, &row.datum_hash) {
                            checks += 1;
                            let got = blake2b_256(bytes);
                            if &got != want {
                                failures.push(format!(
                                    "{}: blake2b-256(inline_datum)={got} but datum_hash={want}",
                                    row.reference
                                ));
                            }
                        }
                    }
                }
                if let Ok(rows) = parse_datums(&f.response) {
                    for row in rows {
                        checks += 1;
                        let got = blake2b_256(&row.bytes);
                        if got != row.datum_hash {
                            failures.push(format!(
                                "datum {}: blake2b-256(bytes)={got}",
                                row.datum_hash
                            ));
                        }
                    }
                }
            }
            "script-blake2b-224-tagged" => {
                for row in parse_scripts(&f.response)? {
                    let (Some(tag), Some(bytes)) = (row.language_tag(), row.bytes.as_ref()) else {
                        continue;
                    };
                    checks += 1;
                    let got = blake2b_224_tagged(tag, bytes);
                    if got != row.script_hash {
                        failures.push(format!(
                            "script {}: blake2b-224({}‖bytes)={got}",
                            row.script_hash, tag
                        ));
                    }
                }
            }
            "tx-body-blake2b-256" => {
                let want = f
                    .identity
                    .get("expect_tx_hash")
                    .and_then(|x| x.as_str())
                    .ok_or_else(|| {
                        eyre!("fixture {stem}: tx-body identity needs `identity.expect_tx_hash`")
                    })?;
                let cbor = parse_tx_cbor(&f.response, want)?;
                // The positive arm: the body's own CBOR hashes to the tx id.
                checks += 1;
                let tx = pallas_traverse::MultiEraTx::decode(&cbor)
                    .map_err(|e| eyre!("fixture {stem}: transaction CBOR does not decode: {e}"))?;
                let body_hash = tx.hash().to_string();
                if body_hash != want {
                    failures.push(format!(
                        "blake2b-256(transaction body CBOR)={body_hash} but tx_hash={want}"
                    ));
                }
                // The negative arm, measured rather than assumed: hashing the
                // WHOLE transaction gives a different value. Without this the
                // positive arm alone would not say *which* bytes the chain
                // hashed.
                checks += 1;
                let whole = blake2b_256(&cbor);
                if whole == want {
                    failures.push(format!(
                        "blake2b-256(whole transaction CBOR)={whole} equals tx_hash, so the \
                         identity does not distinguish the body from the transaction"
                    ));
                }
            }
            "reference-resolution" => {
                // Three readings, each able to fail: the reference scripts'
                // own hashes, the inline datums' own hashes, and the
                // availability of the SPENT rows. The third is the one the
                // whole `hydrated-from-node` route rests on — a node that
                // dropped consumed outputs would make every historic
                // transaction unresolvable — so it is asserted rather than
                // noted.
                let rows = parse_utxos(&f.response)?;
                let mut spent_with_datum = 0usize;
                let mut reference_scripts = 0usize;
                for row in &rows {
                    if let Some(s) = &row.reference_script {
                        reference_scripts += 1;
                        if let (Some(tag), Some(bytes)) = (s.language_tag(), s.bytes.as_ref()) {
                            checks += 1;
                            let got = blake2b_224_tagged(tag, bytes);
                            if got != s.script_hash {
                                failures.push(format!(
                                    "{}: reference script blake2b-224({tag}‖bytes)={got} but \
                                     hash={}",
                                    row.reference, s.script_hash
                                ));
                            }
                        } else {
                            failures.push(format!(
                                "{}: carries a reference script of type `{}` with {} byte(s), so \
                                 its identity cannot be recomputed",
                                row.reference,
                                s.script_type,
                                s.bytes.as_ref().map(|b| b.len()).unwrap_or(0)
                            ));
                        }
                    }
                    if let (Some(bytes), Some(want)) = (&row.inline_datum, &row.datum_hash) {
                        checks += 1;
                        let got = blake2b_256(bytes);
                        if &got != want {
                            failures.push(format!(
                                "{}: blake2b-256(inline_datum)={got} but datum_hash={want}",
                                row.reference
                            ));
                        }
                        if row.is_spent {
                            spent_with_datum += 1;
                        }
                    }
                }
                checks += 1;
                if spent_with_datum == 0 {
                    failures.push(
                        "no row in this capture is both spent AND carrying its inline datum. \
                         That pairing is the measurement the `hydrated-from-node` route depends \
                         on; without it the capture does not show that a historic transaction's \
                         inputs can be resolved at all"
                            .to_string(),
                    );
                }
                // AND it must carry at least one reference script.
                //
                // Found by driving R9 red on 2026-10-31: with the two
                // reference-script rows deleted from this capture, the
                // identity above still reported ANCHORED, because the
                // per-script arm simply had nothing to iterate. An identity
                // that is satisfied by the absence of the thing it is about is
                // the blind instrument this whole convention exists to rule
                // out.
                checks += 1;
                if reference_scripts == 0 {
                    failures.push(
                        "no row in this capture carries a reference script, so the CIP-33 \
                         resolution path is named here and not demonstrated. This identity is \
                         about reference scripts and cannot be satisfied by their absence"
                            .to_string(),
                    );
                }
            }
            "pre-babbage-datum-hash-only" => {
                // The reading this case exists to make, stated as a property
                // that can fail: a pre-Babbage output names its datum by HASH
                // and carries no inline datum (CIP-32 did not exist), and the
                // bytes behind that hash are in the paired /datum_info
                // capture, hashing to it. So the datum's identity is proven
                // across two captures rather than left unchecked because one
                // of them has no datum to hash.
                let datums =
                    parse_datums(&src.load("datum_info_first_plutus_execution")?.response)?;
                for row in parse_utxos(&f.response)? {
                    checks += 1;
                    if row.inline_datum.is_some() {
                        failures.push(format!(
                            "{}: carries an inline datum, which a pre-Babbage output cannot — \
                             this capture is not the era it claims to be",
                            row.reference
                        ));
                        continue;
                    }
                    let Some(want) = &row.datum_hash else {
                        failures.push(format!(
                            "{}: carries neither an inline datum nor a datum_hash, so the \
                             execution that spends it has no datum identity to check at all",
                            row.reference
                        ));
                        continue;
                    };
                    checks += 1;
                    match datums.iter().find(|d| &d.datum_hash == want) {
                        None => failures.push(format!(
                            "{}: names datum_hash {want} and the paired /datum_info capture does \
                             not carry it, so the datum's bytes are unproven",
                            row.reference
                        )),
                        Some(d) => {
                            let got = blake2b_256(&d.bytes);
                            if &got != want {
                                failures.push(format!(
                                    "{}: blake2b-256(datum bytes)={got} but datum_hash={want}",
                                    row.reference
                                ));
                            }
                        }
                    }
                }
            }
            "cross-fixture" => {
                // The two enumeration captures must agree about the same
                // blocks: every block hash present in one, and every row count
                // equal to the other's `tx_count`.
                let blocks = src.load("blocks_alonzo_window")?;
                let txs = src.load("block_txs_alonzo_window")?;
                let blocks = parse_blocks(&blocks.response)?;
                let txs = parse_block_txs(&txs.response)?;
                for b in &blocks {
                    checks += 1;
                    let n = txs.iter().filter(|t| t.block_hash == b.hash).count() as u64;
                    if n != b.tx_count {
                        failures.push(format!(
                            "block {} at height {}: /blocks says tx_count={} and /block_txs \
                             carries {n} row(s)",
                            b.hash, b.block_height, b.tx_count
                        ));
                    }
                }
                for t in &txs {
                    if !blocks.iter().any(|b| b.hash == t.block_hash) {
                        failures.push(format!(
                            "transaction {} names block {}, which /blocks does not carry",
                            t.tx_hash, t.block_hash
                        ));
                    }
                }
            }
            "evaluator-arity" => {
                // `parse_epoch_params` is itself the check: it refuses a cost
                // model that is not a dense index map. Run it for every epoch
                // the capture carries, so a hole is a failure rather than a
                // surprise at replay time.
                let per_epoch = f.response.as_object().ok_or_else(|| {
                    eyre!("fixture {stem}: `response` must be an object keyed by epoch")
                })?;
                for (epoch, raw) in per_epoch {
                    checks += 1;
                    let n: u64 = epoch.parse().unwrap_or(0);
                    if let Err(e) = parse_epoch_params(raw, n) {
                        failures.push(format!("epoch {epoch}: {e}"));
                    }
                }
            }
            "none" => {
                // Stated as unanchored. The one property that IS checkable is
                // internal: epochs ascending, protocol_major non-decreasing.
                if let Ok(rows) = parse_protocol_versions(&f.response) {
                    for w in rows.windows(2) {
                        checks += 1;
                        if w[0].epoch_no >= w[1].epoch_no {
                            failures.push(format!(
                                "epoch {} is not below {}",
                                w[0].epoch_no, w[1].epoch_no
                            ));
                        }
                        if w[0].protocol_major > w[1].protocol_major {
                            failures.push(format!(
                                "protocol_major goes backwards from {} (epoch {}) to {} (epoch {})",
                                w[0].protocol_major,
                                w[0].epoch_no,
                                w[1].protocol_major,
                                w[1].epoch_no
                            ));
                        }
                    }
                }
            }
            other => {
                failures.push(format!(
                    "identity kind `{other}` is not one this verifier knows, so the fixture's \
                     bytes were NOT checked. Adding a kind means adding its reading here"
                ));
            }
        }

        let verdict = if !failures.is_empty() {
            IdentityVerdict::Broken
        } else if f.identity_kind == "none" || f.identity_kind == "evaluator-arity" {
            IdentityVerdict::Unanchored
        } else if checks == 0 {
            IdentityVerdict::Broken
        } else {
            IdentityVerdict::Anchored
        };

        out.push(IdentityReading {
            fixture: stem,
            kind: f.identity_kind,
            verdict,
            checks,
            failures,
            endpoint: f.endpoint,
            fetched: f.fetched,
        });
    }
    Ok(out)
}

/// A one-line-per-fixture rendering of [`verify_identities`].
pub fn render_identities(readings: &[IdentityReading]) -> String {
    let mut s = String::new();
    let mut totals: BTreeMap<&str, usize> = BTreeMap::new();
    for r in readings {
        let key = match r.verdict {
            IdentityVerdict::Anchored => "anchored",
            IdentityVerdict::Unanchored => "unanchored",
            IdentityVerdict::Broken => "broken",
        };
        *totals.entry(key).or_default() += 1;
        s.push_str(&format!(
            "  {:<42} {:<28} {:>3} check(s)  {}\n",
            r.fixture, r.kind, r.checks, r.verdict
        ));
        for f in &r.failures {
            s.push_str(&format!("      FAIL {f}\n"));
        }
    }
    s.push_str(&format!(
        "  -- {} fixture(s): {} anchored, {} unanchored, {} broken\n",
        readings.len(),
        totals.get("anchored").copied().unwrap_or(0),
        totals.get("unanchored").copied().unwrap_or(0),
        totals.get("broken").copied().unwrap_or(0),
    ));
    s
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_fixture_names_the_file_and_does_not_fall_back() {
        let src = FixtureSource::at("/nonexistent/fixtures");
        let err = src
            .load("whatever")
            .expect_err("a missing fixture must error");
        let msg = err.to_string();
        assert!(
            msg.contains("whatever.fixture.json"),
            "the refusal must name the file; got: {msg}"
        );
    }

    #[test]
    fn a_range_outside_the_captured_window_is_reported_not_measured() {
        let src = FixtureSource::at_crate_root();
        let err = src
            .blocks_in_range(1, 2)
            .expect_err("an uncaptured range must be refused");
        let msg = err.to_string();
        assert!(
            msg.contains("NOT MEASURED"),
            "an uncaptured range must say NOT MEASURED rather than return an empty list; got: \
             {msg}"
        );
    }

    #[test]
    fn an_uncaptured_transaction_is_reported_not_measured() {
        let src = FixtureSource::at_crate_root();
        let err = src
            .tx_cbor(&"00".repeat(32))
            .expect_err("an uncaptured transaction must be refused");
        assert!(err.to_string().contains("NOT MEASURED"), "got: {err}");
    }
}
