//! The per-transaction execution-budget oracle: a settled transaction's
//! scripts are re-executed by stepping the chain's CEK machine, and the budget
//! that comes out is compared against the execution units the transaction's
//! own witness set declares.
//!
//! # What this module observes
//!
//! Three inputs, all of which are the chain's bytes rather than this
//! recorder's opinion:
//!
//! 1. **the transaction's raw CBOR**, decoded to a `MintedTx` — which is where
//!    the declared `ExUnits` live, as `transaction_witness_set.redeemer`;
//! 2. **the outputs it consumes and refers to**, hydrated leaf by leaf through
//!    CXP-ADA-2's `/utxo_info` path — one batched read for the whole set, see
//!    [`replay_from_source`] — and turned into `uplc`'s
//!    `ResolvedInput`s here — address, value (lovelace *and* native assets),
//!    datum option and reference script;
//! 3. **the cost model of the epoch the transaction settled in**, built by
//!    [`crate::eras::cost_model_for`] from the parameters that epoch's
//!    `/epoch_params` served.
//!
//! From those it reconstructs Plutus's `ScriptContext` through `uplc`'s own
//! `TxInfoV1`/`TxInfoV2`/`TxInfoV3` and `into_script_context`, applies it in
//! the version-correct order, and **steps** the resulting program through
//! [`crate::cek`] one CEK transition at a time.
//!
//! # Why this is the strongest oracle on any chain in this family
//!
//! The reference figure is produced by the network. A submitter declares the
//! units a redeemer may spend and the ledger validates that execution fit
//! inside them, so the figure in the witness set was not computed by anything
//! in this repository and no recorder bug can move it. See
//! [`BudgetRelation`] for what "agreement" means here, which was **measured
//! rather than assumed** — the honest relation is `replayed <= declared`, and
//! this module reports which of equality / under / over each redeemer is.
//!
//! # The application order differs by Plutus version, and getting it wrong
//! produces a trace that still runs
//!
//! Mirrored from `uplc`'s `tx::eval::do_eval_redeemer`:
//!
//! - `ScriptContext::V1V2` — apply the datum (when the execution has one),
//!   then the redeemer, then the context;
//! - `ScriptContext::V3` — apply **only** the context, because a V3 script
//!   context already carries the redeemer and the datum inside it.
//!
//! A swap is a real hazard and it is caught, but **not always in the way the
//! obvious wording suggests, and the difference was measured rather than
//! assumed.** On the corpus case it was driven against
//! (`tests/test_budget_oracle.rs`), applying the V1V2 sequence to a V3 script
//! did *not* run to completion with a wrong budget: the machine failed at step
//! 63 with `EmptyList`, because the extra argument shifts what the script
//! destructures. So on that case the swap is caught by the evaluator, not by
//! the budget. The budget comparison is the arm that catches a swap that *does*
//! run, and whether a given script does is a property of the script. The test
//! asserts "caught, either way" and prints which, rather than claiming a
//! failure mode it did not observe.
//!
//! # What this module does NOT do
//!
//! - **It does not write a trace.** It produces budget readings and step
//!   counts; emitting CTFS is not its job and no `.ct` artefact is created
//!   anywhere in this path.
//! - **It does not fetch.** Every byte arrives through a
//!   [`crate::chain::ChainSource`], which offline is the committed Koios
//!   captures.
//! - **It does not invoke the Aiken compiler and does not touch
//!   [`crate::tracer`].** The script it runs is the UPLC the chain stores,
//!   taken from the witness set or from a CIP-33 reference output; the
//!   source-simulating path is untouched and unreachable from here. Compiling
//!   Aiken source to UPLC remains CXP-ADA-1 deliverable 3.
//! - **It does not pad or substitute a cost model.** An epoch whose parameter
//!   vector is shorter than this evaluator reads is refused by
//!   [`crate::eras::cost_model_for`], and that refusal is propagated as NOT
//!   MEASURED rather than charged under some other era's prices.
//! - **It does not run phase-one validation.** `uplc`'s `eval_phase_one`
//!   checks a submitted transaction's consistency; this transaction is
//!   settled, so the chain already ran it.

use std::collections::BTreeMap;
use std::fmt;

use eyre::{eyre, Context, Result};
use pallas_addresses::{Address, ByronAddress};
use pallas_codec::minicbor;
use pallas_codec::utils::{Bytes, CborWrap, NonEmptyKeyValuePairs, PositiveCoin};
use pallas_primitives::conway::{
    AssetName, DatumOption, ExUnits, Language, MintedTx, PlutusData, PolicyId,
    PostAlonzoTransactionOutput, PseudoScript, Redeemer, ScriptRef, TransactionInput,
    TransactionOutput, Value,
};
use pallas_primitives::Fragment;
use pallas_primitives::PlutusScript as PlutusScriptBytes;
use pallas_traverse::ComputeHash;
use uplc::ast::{FakeNamedDeBruijn, NamedDeBruijn, Program};
use uplc::machine::cost_model::ExBudget;
use uplc::tx::iter_redeemers;
use uplc::tx::script_context::{
    find_script, DataLookupTable, PlutusScript, ResolvedInput, ScriptContext, SlotConfig, TxInfoV1,
    TxInfoV2, TxInfoV3,
};
use uplc::tx::to_plutus_data::ToPlutusData;

use crate::cek::{self, AccountingFault, CekRun};
use crate::chain::{ChainSource, EpochParams, UtxoRef, UtxoRow};
use crate::eras::cost_model_for;

// ---------------------------------------------------------------------------
// The relation
// ---------------------------------------------------------------------------

/// How a replayed budget stands to the one the transaction declares.
///
/// **This enum exists because the relation was measured and is not equality in
/// general.** A Cardano submitter chooses the `ExUnits` it declares — usually
/// by simulating the script — and the ledger's phase-two rule is that
/// execution must *fit inside* the declaration. So the honest statement is
/// `replayed <= declared` on both axes, and the recorder reports which of the
/// three cases each redeemer is rather than asserting the one it hoped for.
///
/// [`BudgetRelation::Over`] is the one that is always a defect: a replay that
/// spends more than the declaration would not have validated on chain, so
/// either the replay is wrong or the transaction could not have settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetRelation {
    /// Replayed exactly what the transaction declares, on both axes.
    Equal,
    /// Replayed strictly less than declared on at least one axis and no more
    /// on either: the ledger's `<=` rule held with slack.
    Under { mem: u64, steps: u64 },
    /// Replayed more than declared on at least one axis. Always a defect.
    Over { mem: i64, steps: i64 },
}

impl BudgetRelation {
    /// Whether the ledger's own rule — execution fits inside the declaration —
    /// held for this redeemer.
    pub fn fits(&self) -> bool {
        !matches!(self, BudgetRelation::Over { .. })
    }

    /// Whether the two figures are identical on both axes.
    pub fn is_exact(&self) -> bool {
        matches!(self, BudgetRelation::Equal)
    }
}

impl fmt::Display for BudgetRelation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BudgetRelation::Equal => write!(f, "EQUAL"),
            BudgetRelation::Under { mem, steps } => {
                write!(f, "UNDER by mem={mem} steps={steps}")
            }
            BudgetRelation::Over { mem, steps } => {
                write!(f, "OVER by mem={mem} steps={steps} -- DEFECT")
            }
        }
    }
}

/// Compare a replayed budget against a declared one.
pub fn relate(replayed: ExBudget, declared: ExUnits) -> BudgetRelation {
    let d_mem = declared.mem as i64;
    let d_steps = declared.steps as i64;
    if replayed.mem > d_mem || replayed.cpu > d_steps {
        return BudgetRelation::Over {
            mem: replayed.mem - d_mem,
            steps: replayed.cpu - d_steps,
        };
    }
    if replayed.mem == d_mem && replayed.cpu == d_steps {
        return BudgetRelation::Equal;
    }
    BudgetRelation::Under {
        mem: (d_mem - replayed.mem) as u64,
        steps: (d_steps - replayed.cpu) as u64,
    }
}

// ---------------------------------------------------------------------------
// Readings
// ---------------------------------------------------------------------------

/// Which arguments were applied to the script, and in which order.
///
/// Recorded per redeemer rather than inferred from the language, so a report
/// shows the V1V2-vs-V3 difference instead of a reader having to trust that it
/// was honoured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Application {
    /// `((script datum) redeemer) context`
    V1V2WithDatum,
    /// `(script redeemer) context`
    V1V2WithoutDatum,
    /// `script context` — the V3 context carries the redeemer and the datum.
    V3ContextOnly,
}

impl fmt::Display for Application {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Application::V1V2WithDatum => write!(f, "((script datum) redeemer) context"),
            Application::V1V2WithoutDatum => write!(f, "(script redeemer) context"),
            Application::V3ContextOnly => write!(f, "script context"),
        }
    }
}

/// One redeemer's budget reading.
#[derive(Debug, Clone)]
pub struct RedeemerBudget {
    /// `spend` / `mint` / `cert` / `reward` / `vote` / `propose`.
    pub purpose: &'static str,
    pub index: u32,
    /// `PlutusV1` / `PlutusV2` / `PlutusV3`, from which script was found.
    pub language: &'static str,
    /// The script's hash, recomputed from the bytes that were executed.
    pub script_hash: String,
    pub application: Application,
    /// Units the transaction's witness set declares for this redeemer.
    pub declared: ExUnits,
    /// Units the stepped replay consumed.
    pub replayed: ExBudget,
    pub relation: BudgetRelation,
    /// CEK transitions observed.
    pub steps: usize,
    /// Whether the script evaluated to `Term::Error`. A settled transaction's
    /// phase-two scripts succeeded, so `true` here is a divergence even when
    /// the budget happens to fit.
    pub errored: bool,
}

/// Every reading for one transaction.
#[derive(Debug, Clone)]
pub struct TxBudgetReading {
    pub tx_hash: String,
    /// The epoch the transaction settled in, whose cost model was charged.
    pub epoch_no: u64,
    /// That epoch's ledger era, as `/epoch_params` reports it.
    pub ledger_era: Option<String>,
    /// How many `/utxo_info` leaves were resolved to build the contexts.
    pub leaves_resolved: usize,
    pub redeemers: Vec<RedeemerBudget>,
}

impl TxBudgetReading {
    /// Every redeemer's replay fits inside its declaration, and none errored.
    pub fn agrees(&self) -> bool {
        !self.redeemers.is_empty()
            && self
                .redeemers
                .iter()
                .all(|r| r.relation.fits() && !r.errored)
    }

    /// Redeemers whose replay matched the declaration exactly.
    pub fn exact_count(&self) -> usize {
        self.redeemers
            .iter()
            .filter(|r| r.relation.is_exact())
            .count()
    }

    /// Redeemers whose replay exceeded the declaration — always defects.
    pub fn over_count(&self) -> usize {
        self.redeemers.iter().filter(|r| !r.relation.fits()).count()
    }
}

// ---------------------------------------------------------------------------
// Resolved inputs
// ---------------------------------------------------------------------------

/// Turn a hydrated `/utxo_info` row into the `ResolvedInput` the evaluator
/// consumes.
///
/// Every field is load-bearing for the script context, so nothing is defaulted
/// when it cannot be read: a wrong address or a dropped native asset builds a
/// `TxOut` the chain never hashed, and the budget computed from it would still
/// look like a budget.
pub fn resolved_input(row: &UtxoRow) -> Result<ResolvedInput> {
    let input = TransactionInput {
        transaction_id: parse_hash32(&row.reference.tx_hash)
            .with_context(|| format!("output reference {}", row.reference))?,
        index: row.reference.index,
    };

    let address_bytes = address_bytes(&row.address)
        .with_context(|| format!("output {} address `{}`", row.reference, row.address))?;

    let datum_option = match (&row.inline_datum, &row.datum_hash) {
        // CIP-32: the datum travels on the output itself.
        (Some(bytes), _) => {
            let data = PlutusData::decode_fragment(bytes).map_err(|e| {
                eyre!(
                    "output {}: inline datum is not decodable PlutusData: {e:?}",
                    row.reference
                )
            })?;
            Some(DatumOption::Data(CborWrap(data)))
        }
        // Pre-Babbage: the output names its datum by hash and the datum
        // itself lives in some transaction's witness set.
        (None, Some(h)) => {
            Some(DatumOption::Hash(parse_hash32(h).with_context(|| {
                format!("output {} datum_hash", row.reference)
            })?))
        }
        (None, None) => None,
    };

    let script_ref = match &row.reference_script {
        None => None,
        Some(s) => {
            let bytes = s.bytes.as_ref().ok_or_else(|| {
                eyre!(
                    "output {} carries reference script {} with no bytes, so the script it \
                     resolves cannot be executed. A reference script reported without its bytes \
                     is an unresolved requisite, not an empty one",
                    row.reference,
                    s.script_hash
                )
            })?;
            Some(CborWrap(script_ref_of(&s.script_type, bytes).ok_or_else(
                || {
                    eyre!(
                        "output {} carries reference script {} of type `{}`, which is not a \
                         Plutus language this evaluator runs",
                        row.reference,
                        s.script_hash,
                        s.script_type
                    )
                },
            )?))
        }
    };

    Ok(ResolvedInput {
        input,
        output: TransactionOutput::PostAlonzo(PostAlonzoTransactionOutput {
            address: Bytes::from(address_bytes),
            value: value_of(row)?,
            datum_option,
            script_ref,
        }),
    })
}

/// Every row, as a `ResolvedInput`.
pub fn resolved_inputs(rows: &[UtxoRow]) -> Result<Vec<ResolvedInput>> {
    rows.iter().map(resolved_input).collect()
}

fn value_of(row: &UtxoRow) -> Result<Value> {
    if row.assets.is_empty() {
        return Ok(Value::Coin(row.value_lovelace));
    }
    // Group by policy. The evaluator sorts a value canonically before hashing
    // it into a script context, so this grouping decides the *contents* and
    // not the encoding.
    let mut by_policy: BTreeMap<PolicyId, Vec<(AssetName, PositiveCoin)>> = BTreeMap::new();
    for a in &row.assets {
        let policy: PolicyId = parse_hash28(&a.policy_id)
            .with_context(|| format!("output {} asset policy id", row.reference))?;
        let name = AssetName::from(hex::decode(&a.asset_name).map_err(|e| {
            eyre!(
                "output {}: asset name `{}` is not hex: {e}",
                row.reference,
                a.asset_name
            )
        })?);
        let qty = PositiveCoin::try_from(a.quantity).map_err(|e| {
            eyre!(
                "output {}: asset {}.{} has quantity {} which is not a positive coin: {e}. An \
                 output cannot hold zero of an asset, so this row is not something the chain \
                 served",
                row.reference,
                a.policy_id,
                a.asset_name,
                a.quantity
            )
        })?;
        by_policy.entry(policy).or_default().push((name, qty));
    }
    let multiasset: Vec<(PolicyId, NonEmptyKeyValuePairs<AssetName, PositiveCoin>)> = by_policy
        .into_iter()
        .map(|(p, names)| (p, NonEmptyKeyValuePairs::Indef(names)))
        .collect();
    Ok(Value::Multiasset(
        row.value_lovelace,
        NonEmptyKeyValuePairs::Indef(multiasset),
    ))
}

/// Raw bytes of an address, from whichever encoding the node used.
///
/// Shelley-era and later addresses are bech32; Byron addresses are base58. A
/// script context carries the raw bytes, so both are decoded rather than one
/// being assumed.
fn address_bytes(addr: &str) -> Result<Vec<u8>> {
    if let Ok(a) = Address::from_bech32(addr) {
        return Ok(a.to_vec());
    }
    if let Ok(a) = ByronAddress::from_base58(addr) {
        return Ok(a.to_vec());
    }
    Err(eyre!(
        "address `{addr}` decodes as neither bech32 (Shelley and later) nor base58 (Byron)"
    ))
}

fn script_ref_of(script_type: &str, bytes: &[u8]) -> Option<ScriptRef> {
    match script_type {
        "plutusV1" => Some(PseudoScript::PlutusV1Script(PlutusScriptBytes(
            Bytes::from(bytes.to_vec()),
        ))),
        "plutusV2" => Some(PseudoScript::PlutusV2Script(PlutusScriptBytes(
            Bytes::from(bytes.to_vec()),
        ))),
        "plutusV3" => Some(PseudoScript::PlutusV3Script(PlutusScriptBytes(
            Bytes::from(bytes.to_vec()),
        ))),
        _ => None,
    }
}

fn parse_hash32(hex_str: &str) -> Result<pallas_crypto::hash::Hash<32>> {
    let raw = hex::decode(hex_str).map_err(|e| eyre!("`{hex_str}` is not hex: {e}"))?;
    let arr: [u8; 32] = raw
        .try_into()
        .map_err(|_| eyre!("`{hex_str}` is not 32 bytes"))?;
    Ok(pallas_crypto::hash::Hash::from(arr))
}

fn parse_hash28(hex_str: &str) -> Result<pallas_crypto::hash::Hash<28>> {
    let raw = hex::decode(hex_str).map_err(|e| eyre!("`{hex_str}` is not hex: {e}"))?;
    let arr: [u8; 28] = raw
        .try_into()
        .map_err(|_| eyre!("`{hex_str}` is not 28 bytes"))?;
    Ok(pallas_crypto::hash::Hash::from(arr))
}

// ---------------------------------------------------------------------------
// The replay
// ---------------------------------------------------------------------------

fn purpose_name(tag: pallas_primitives::conway::RedeemerTag) -> &'static str {
    use pallas_primitives::conway::RedeemerTag as T;
    match tag {
        T::Spend => "spend",
        T::Mint => "mint",
        T::Cert => "cert",
        T::Reward => "reward",
        T::Vote => "vote",
        T::Propose => "propose",
    }
}

/// Step one transaction's scripts and compare each redeemer's budget against
/// the units its witness set declares.
///
/// `params` must be the parameters of the epoch the transaction settled in:
/// charging a transaction under another era's prices is not a replay of it.
pub fn replay_tx(
    tx_cbor: &[u8],
    expect_tx_hash: &str,
    rows: &[UtxoRow],
    params: &EpochParams,
    slot_config: &SlotConfig,
    fault: AccountingFault,
) -> Result<TxBudgetReading> {
    let tx: MintedTx = minicbor::decode(tx_cbor).map_err(|e| {
        eyre!(
            "transaction {expect_tx_hash}: the committed CBOR does not decode as a Conway-format \
             transaction: {e}. Post-Alonzo formats are forward-compatible enough that every \
             mainnet Plutus transaction decodes this way; a failure here is a different kind of \
             transaction and not a cost-model question"
        )
    })?;

    let utxos = resolved_inputs(rows)?;
    let lookup_table = DataLookupTable::from_transaction(&tx, &utxos);

    let redeemers = tx
        .transaction_witness_set
        .redeemer
        .as_ref()
        .ok_or_else(|| {
            eyre!(
                "transaction {expect_tx_hash} carries no redeemers, so it performed no script \
                 execution and there is no budget to compare"
            )
        })?;

    // Mirrors `uplc::tx::eval_phase_two`: each redeemer starts from what the
    // previous ones left, so a replay is sequenced the way the ledger
    // sequences it rather than each script being handed the whole budget.
    let mut remaining = ExBudget::default();
    let mut out: Vec<RedeemerBudget> = Vec::new();

    for (key, data, declared) in iter_redeemers(redeemers) {
        let redeemer = Redeemer {
            tag: key.tag,
            index: key.index,
            data: data.clone(),
            ex_units: declared,
        };

        let reading = replay_one(
            &tx,
            &utxos,
            &lookup_table,
            &redeemer,
            params,
            slot_config,
            remaining,
            fault,
        )
        .with_context(|| {
            format!(
                "transaction {expect_tx_hash}: {} redeemer at index {}",
                purpose_name(key.tag),
                key.index
            )
        })?;

        remaining.cpu -= reading.replayed.cpu;
        remaining.mem -= reading.replayed.mem;
        out.push(reading);
    }

    Ok(TxBudgetReading {
        tx_hash: expect_tx_hash.to_string(),
        epoch_no: params.epoch_no,
        ledger_era: params.era.clone(),
        leaves_resolved: rows.len(),
        redeemers: out,
    })
}

#[allow(clippy::too_many_arguments)]
fn replay_one(
    tx: &MintedTx,
    utxos: &[ResolvedInput],
    lookup_table: &DataLookupTable,
    redeemer: &Redeemer,
    params: &EpochParams,
    slot_config: &SlotConfig,
    initial_budget: ExBudget,
    fault: AccountingFault,
) -> Result<RedeemerBudget> {
    let (script, datum) = find_script(redeemer, tx, utxos, lookup_table).map_err(|e| {
        eyre!(
            "the script this redeemer runs could not be located: {e:?}. Every script a settled \
             transaction executed is either in its witness set or on a CIP-33 reference output, \
             so this says a requisite was not hydrated rather than that the chain is missing one"
        )
    })?;

    // The script hash is recomputed from the bytes that are about to be
    // executed, not taken from whatever named them. `blake2b-224(language
    // tag ‖ bytes)` is the chain's own script identity, and `compute_hash`
    // on a versioned `PlutusScript<N>` is that computation — which is why
    // the arms carry the version in the type rather than in a string.
    let (language, lang_name, script_bytes, script_hash) = match &script {
        PlutusScript::V1(b) => (
            Language::PlutusV1,
            "PlutusV1",
            b.0.to_vec(),
            b.compute_hash().to_string(),
        ),
        PlutusScript::V2(b) => (
            Language::PlutusV2,
            "PlutusV2",
            b.0.to_vec(),
            b.compute_hash().to_string(),
        ),
        PlutusScript::V3(b) => (
            Language::PlutusV3,
            "PlutusV3",
            b.0.to_vec(),
            b.compute_hash().to_string(),
        ),
    };

    // The era's own prices. `cost_model_for` refuses a vector shorter than
    // this evaluator reads rather than padding it, and that refusal is
    // propagated: a budget charged under invented prices would be a number,
    // not a measurement.
    let costs = cost_model_for(params, lang_name).with_context(|| {
        format!(
            "this transaction settled in epoch {}, and a budget comparison is only a reading when \
             the replay is charged under that epoch's own prices",
            params.epoch_no
        )
    })?;

    let mut buffer = Vec::new();
    let program: Program<NamedDeBruijn> =
        Program::<FakeNamedDeBruijn>::from_cbor(&script_bytes, &mut buffer)
            .map_err(|e| eyre!("script {script_hash} does not decode as flat UPLC: {e:?}"))?
            .into();

    // The script context, from `uplc`'s own reconstruction. The version picks
    // which `TxInfo` shape is built, because V1 forbids what V2 allows and V3
    // carries fields neither has.
    let tx_info = match language {
        Language::PlutusV1 => TxInfoV1::from_transaction(tx, utxos, slot_config),
        Language::PlutusV2 => TxInfoV2::from_transaction(tx, utxos, slot_config),
        Language::PlutusV3 => TxInfoV3::from_transaction(tx, utxos, slot_config),
    }
    .map_err(|e| eyre!("the {lang_name} transaction info could not be assembled: {e:?}"))?;

    let script_context = tx_info
        .into_script_context(redeemer, datum.as_ref())
        .ok_or_else(|| {
            eyre!(
                "no script purpose in the reconstructed transaction info matches this redeemer, \
                 so no script context can be built for it"
            )
        })?;

    // Mirrors `uplc::tx::eval::do_eval_redeemer` exactly. The V1V2 and V3 arms
    // differ and a swap produces a program that evaluates and is wrong.
    let (program, application) = match script_context {
        ScriptContext::V1V2 { .. } => {
            let (program, application) = match &datum {
                Some(d) => (program.apply_data(d.clone()), Application::V1V2WithDatum),
                None => (program, Application::V1V2WithoutDatum),
            };
            (
                program
                    .apply_data(redeemer.data.clone())
                    .apply_data(script_context.to_plutus_data()),
                application,
            )
        }
        ScriptContext::V3 { .. } => (
            program.apply_data(script_context.to_plutus_data()),
            Application::V3ContextOnly,
        ),
    };

    // Stepped, not evaluated: this is the whole point of the milestone. The
    // machine is the chain's own and every figure below comes off it.
    let run: CekRun =
        cek::step_program_with_fault(program, &language, costs, initial_budget, fault)?;

    let replayed = run.consumed();
    let declared = redeemer.ex_units;

    Ok(RedeemerBudget {
        purpose: purpose_name(redeemer.tag),
        index: redeemer.index,
        language: lang_name,
        script_hash,
        application,
        declared,
        replayed,
        relation: relate(replayed, declared),
        steps: run.step_count(),
        errored: matches!(run.result, uplc::ast::Term::Error),
    })
}

// ---------------------------------------------------------------------------
// Driving it from a chain source
// ---------------------------------------------------------------------------

/// Replay one transaction end to end against a [`ChainSource`].
///
/// Reads the transaction's CBOR, hydrates every output it consumes or refers
/// to in **one** batched leaf read, takes the epoch's parameters, and steps.
/// The epoch is supplied rather than discovered because a bare transaction's
/// bytes do not say which epoch settled them — see
/// [`crate::chain::epoch_of_tx`].
pub fn replay_from_source(
    src: &dyn ChainSource,
    tx_hash: &str,
    epoch_no: u64,
    fault: AccountingFault,
) -> Result<TxBudgetReading> {
    let cbor = src.tx_cbor(tx_hash)?;
    let tx: MintedTx = minicbor::decode(&cbor)
        .map_err(|e| eyre!("transaction {tx_hash}: CBOR does not decode as Conway format: {e}"))?;

    let mut wanted: Vec<UtxoRef> = tx
        .transaction_body
        .inputs
        .iter()
        .map(|i| UtxoRef::new(i.transaction_id.to_string(), i.index))
        .collect();
    if let Some(refs) = tx.transaction_body.reference_inputs.as_ref() {
        for i in refs.iter() {
            wanted.push(UtxoRef::new(i.transaction_id.to_string(), i.index));
        }
    }

    let rows = src.utxos(&wanted)?;
    let missing: Vec<String> = wanted
        .iter()
        .filter(|r| !rows.iter().any(|row| &row.reference == *r))
        .map(|r| r.to_string())
        .collect();
    if !missing.is_empty() {
        return Err(eyre!(
            "transaction {tx_hash}: {} of {} output(s) it consumes or refers to were not \
             resolved ({}). Plutus applies a script context built from every resolved input, so \
             a partially hydrated transaction is NOT MEASURED rather than measured against a \
             shorter context",
            missing.len(),
            wanted.len(),
            missing.join(", ")
        ));
    }

    let params = src.epoch_params(epoch_no)?;
    replay_tx(
        &cbor,
        tx_hash,
        &rows,
        &params,
        // `uplc`'s default is MAINNET's Shelley-era slot configuration —
        // `zero_slot: 4_492_800`, `zero_time: 1_596_059_091_000`,
        // `slot_length: 1000` — and it is load-bearing rather than incidental:
        // it converts the transaction's validity *slot* range into the POSIX
        // *time* range the script context carries, so a wrong configuration
        // would put a different interval in the context and change the
        // budget. That it is right for this chain is a consequence of the
        // measurement rather than an assumption: five mainnet transactions
        // across two eras reproduced the chain's units exactly under it.
        // A testnet transaction would need its own, and `replay_tx` takes one
        // for that reason.
        &SlotConfig::default(),
        fault,
    )
}

// ---------------------------------------------------------------------------
// The census: the relation, over a population
// ---------------------------------------------------------------------------

/// Why one transaction yielded no budget reading.
///
/// Kept as a row rather than folded into a count, because the two reasons are
/// different findings: an epoch whose cost model this evaluator cannot be
/// given is **NOT MEASURED** and says so, while a transaction that could not
/// be replayed for any other reason is a defect to look at.
#[derive(Debug, Clone)]
pub struct NotMeasured {
    pub tx_hash: String,
    pub epoch_no: Option<u64>,
    pub why: String,
}

/// The relation between replayed and declared budgets, over a population.
#[derive(Debug, Clone, Default)]
pub struct Census {
    /// Transactions whose bytes were read.
    pub transactions_read: usize,
    /// Of those, the ones carrying at least one redeemer.
    pub transactions_with_scripts: usize,
    pub readings: Vec<TxBudgetReading>,
    pub not_measured: Vec<NotMeasured>,
}

impl Census {
    pub fn redeemer_count(&self) -> usize {
        self.readings.iter().map(|r| r.redeemers.len()).sum()
    }

    pub fn exact_count(&self) -> usize {
        self.readings.iter().map(|r| r.exact_count()).sum()
    }

    pub fn under_count(&self) -> usize {
        self.readings
            .iter()
            .flat_map(|r| &r.redeemers)
            .filter(|b| matches!(b.relation, BudgetRelation::Under { .. }))
            .count()
    }

    pub fn over_count(&self) -> usize {
        self.readings.iter().map(|r| r.over_count()).sum()
    }

    pub fn errored_count(&self) -> usize {
        self.readings
            .iter()
            .flat_map(|r| &r.redeemers)
            .filter(|b| b.errored)
            .count()
    }

    /// The largest `UNDER` slack seen, as `(mem, steps)`. Zero when every
    /// reading was exact — which is the measured state, and the figure is kept
    /// so a change away from it is visible rather than inferred.
    pub fn max_under(&self) -> (u64, u64) {
        self.readings
            .iter()
            .flat_map(|r| &r.redeemers)
            .fold((0, 0), |(m, s), b| match b.relation {
                BudgetRelation::Under { mem, steps } => (m.max(mem), s.max(steps)),
                _ => (m, s),
            })
    }
}

/// Replay every named transaction and collect the relation.
///
/// A transaction whose era this evaluator cannot be given a cost model for is
/// recorded as NOT MEASURED with the refusal verbatim, not skipped and not
/// charged under some other era's prices.
pub fn census_over_hashes(
    src: &dyn ChainSource,
    tx_hashes: &[String],
    fault: AccountingFault,
) -> Census {
    let mut census = Census::default();
    for h in tx_hashes {
        census.transactions_read += 1;
        let epoch = match src.tx_epoch(h) {
            Ok(e) => e,
            Err(e) => {
                census.not_measured.push(NotMeasured {
                    tx_hash: h.clone(),
                    epoch_no: None,
                    why: format!("{e:#}"),
                });
                continue;
            }
        };
        match replay_from_source(src, h, epoch, fault) {
            Ok(r) => {
                census.transactions_with_scripts += 1;
                census.readings.push(r);
            }
            Err(e) => {
                let msg = format!("{e:#}");
                // A transaction with no redeemers performed no script
                // execution; it is not a measurement failure and must not be
                // counted as one.
                if msg.contains("carries no redeemers") {
                    continue;
                }
                census.not_measured.push(NotMeasured {
                    tx_hash: h.clone(),
                    epoch_no: Some(epoch),
                    why: msg,
                });
            }
        }
    }
    census
}

/// Every transaction in a height range that performed a script execution,
/// replayed.
///
/// This is the live census surface: it enumerates the range, reads each
/// transaction's bytes, and replays the ones carrying redeemers. Offline it
/// works only for a range the captures cover, and says so rather than
/// enumerating nothing.
pub fn census_over_range(
    src: &dyn ChainSource,
    lo: u64,
    hi: u64,
    fault: AccountingFault,
) -> Result<Census> {
    let blocks = src.blocks_in_range(lo, hi)?;
    let hashes: Vec<String> = blocks.iter().map(|b| b.hash.clone()).collect();
    let rows = src.block_txs(&hashes)?;

    let mut census = Census::default();
    for row in &rows {
        census.transactions_read += 1;
        let cbor = match src.tx_cbor(&row.tx_hash) {
            Ok(c) => c,
            Err(e) => {
                census.not_measured.push(NotMeasured {
                    tx_hash: row.tx_hash.clone(),
                    epoch_no: Some(row.epoch_no),
                    why: format!("{e:#}"),
                });
                continue;
            }
        };
        // Skip the transactions with no script execution before spending a
        // leaf read on them: most of a block is payments.
        let has_redeemers = match minicbor::decode::<MintedTx>(&cbor) {
            Ok(tx) => tx.transaction_witness_set.redeemer.is_some(),
            Err(e) => {
                census.not_measured.push(NotMeasured {
                    tx_hash: row.tx_hash.clone(),
                    epoch_no: Some(row.epoch_no),
                    why: format!("CBOR does not decode as Conway format: {e}"),
                });
                continue;
            }
        };
        if !has_redeemers {
            continue;
        }
        match replay_from_source(src, &row.tx_hash, row.epoch_no, fault) {
            Ok(r) => {
                census.transactions_with_scripts += 1;
                census.readings.push(r);
            }
            Err(e) => census.not_measured.push(NotMeasured {
                tx_hash: row.tx_hash.clone(),
                epoch_no: Some(row.epoch_no),
                why: format!("{e:#}"),
            }),
        }
    }
    Ok(census)
}

/// Render a census: the distribution, and every NOT MEASURED row named.
pub fn render_census(c: &Census) -> String {
    let mut s = String::new();
    for r in &c.readings {
        s.push_str(&render(r));
    }
    let (mem_slack, step_slack) = c.max_under();
    s.push_str(&format!(
        "\nthe relation between the replayed budget and the one the chain records:\n  \
         {} transaction(s) read, {} with a script execution, {} redeemer(s) replayed\n  \
         EQUAL  {}\n  UNDER  {} (largest slack mem={mem_slack} steps={step_slack})\n  \
         OVER   {}  <- always a defect\n  script evaluated to ERROR: {}\n",
        c.transactions_read,
        c.transactions_with_scripts,
        c.redeemer_count(),
        c.exact_count(),
        c.under_count(),
        c.over_count(),
        c.errored_count(),
    ));
    if c.not_measured.is_empty() {
        s.push_str("  NOT MEASURED: none\n");
    } else {
        s.push_str(&format!(
            "  NOT MEASURED: {} transaction(s)\n",
            c.not_measured.len()
        ));
        for n in &c.not_measured {
            s.push_str(&format!(
                "    {} (epoch {}): {}\n",
                n.tx_hash,
                n.epoch_no
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "unknown".into()),
                n.why
            ));
        }
    }
    s
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render one transaction's budget reading.
pub fn render(r: &TxBudgetReading) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "transaction {}\n  epoch {} (era {}), {} hydrated leaf/leaves, {} redeemer(s)\n",
        r.tx_hash,
        r.epoch_no,
        r.ledger_era.as_deref().unwrap_or("unknown"),
        r.leaves_resolved,
        r.redeemers.len()
    ));
    for b in &r.redeemers {
        s.push_str(&format!(
            "  {}[{}] {} script {}\n    applied {}\n    declared mem={} steps={}\n    \
             replayed mem={} steps={}  ({} CEK transitions)\n    {}{}\n",
            b.purpose,
            b.index,
            b.language,
            b.script_hash,
            b.application,
            b.declared.mem,
            b.declared.steps,
            b.replayed.mem,
            b.replayed.cpu,
            b.steps,
            b.relation,
            if b.errored {
                " -- the script evaluated to ERROR, which a settled transaction's did not"
            } else {
                ""
            }
        ));
    }
    s.push_str(&format!(
        "  -- {} of {} exact, {} over the declaration\n",
        r.exact_count(),
        r.redeemers.len(),
        r.over_count()
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
    fn the_relation_names_equality_under_and_over_separately() {
        let declared = ExUnits {
            mem: 100,
            steps: 1000,
        };
        assert_eq!(
            relate(
                ExBudget {
                    mem: 100,
                    cpu: 1000
                },
                declared
            ),
            BudgetRelation::Equal
        );
        assert_eq!(
            relate(ExBudget { mem: 90, cpu: 1000 }, declared),
            BudgetRelation::Under { mem: 10, steps: 0 }
        );
        // One axis over is over, even when the other is under: the ledger's
        // rule is per axis, so a combined figure would hide a defect.
        match relate(ExBudget { mem: 90, cpu: 1001 }, declared) {
            BudgetRelation::Over { mem, steps } => {
                assert_eq!((mem, steps), (-10, 1));
            }
            other => panic!("one axis over must read as OVER; got {other}"),
        }
    }

    #[test]
    fn over_does_not_fit_and_under_does() {
        assert!(BudgetRelation::Under { mem: 1, steps: 1 }.fits());
        assert!(BudgetRelation::Equal.fits());
        assert!(!BudgetRelation::Over { mem: 1, steps: 0 }.fits());
        assert!(!BudgetRelation::Under { mem: 1, steps: 0 }.is_exact());
    }

    fn row(value: u64, assets: Vec<crate::chain::AssetRow>) -> UtxoRow {
        UtxoRow {
            reference: UtxoRef::new(
                "7510385def92ad50602b75e519f733ed2acd248610aa5f920218ce8010acd4b4",
                0,
            ),
            is_spent: true,
            address: "addr1w984jz3aszhqxy466zmy64qv8l6ssrnhy58fm06sz93sq9s9edd6l".into(),
            value_lovelace: value,
            assets,
            datum_hash: None,
            inline_datum: None,
            reference_script: None,
        }
    }

    #[test]
    fn an_output_with_no_assets_is_a_plain_coin_value() {
        match value_of(&row(7_000_000, vec![])).unwrap() {
            Value::Coin(c) => assert_eq!(c, 7_000_000),
            other => panic!("a bare lovelace output must not become a multiasset: {other:?}"),
        }
    }

    #[test]
    fn two_assets_of_one_policy_group_under_that_policy() {
        // Measured on mainnet: output 9a306720…#0 holds two assets of the
        // SAME policy. Emitting them as two policy entries would build a
        // TxOut the chain never hashed while still looking like a value.
        let p = "97e22b8e5dbadc969a5008934350a974629e74e64b33138c36531d92";
        let assets = vec![
            crate::chain::AssetRow {
                policy_id: p.into(),
                asset_name: "0014df10".into(),
                quantity: 998_417_939_681_556,
            },
            crate::chain::AssetRow {
                policy_id: p.into(),
                asset_name: "706f6f6c".into(),
                quantity: 1,
            },
        ];
        match value_of(&row(8_489_607, assets)).unwrap() {
            Value::Multiasset(coin, ma) => {
                assert_eq!(coin, 8_489_607);
                assert_eq!(ma.len(), 1, "one policy, not two");
                assert_eq!(ma[0].1.len(), 2, "both asset names under that policy");
            }
            other => panic!("expected a multiasset value, got {other:?}"),
        }
    }

    #[test]
    fn a_zero_quantity_asset_is_refused_rather_than_dropped() {
        // An output cannot hold zero of an asset. Dropping the row would
        // silently build a different TxOut; refusing says the row is not
        // something the chain served.
        let assets = vec![crate::chain::AssetRow {
            policy_id: "97e22b8e5dbadc969a5008934350a974629e74e64b33138c36531d92".into(),
            asset_name: "".into(),
            quantity: 0,
        }];
        let err = value_of(&row(1, assets)).expect_err("zero is not a positive coin");
        assert!(
            err.to_string().contains("positive coin"),
            "the refusal must say why; got: {err}"
        );
    }

    #[test]
    fn a_bech32_and_a_base58_address_both_decode() {
        // Shelley-era bech32.
        assert_eq!(
            address_bytes("addr1w984jz3aszhqxy466zmy64qv8l6ssrnhy58fm06sz93sq9s9edd6l")
                .unwrap()
                .len(),
            29,
            "a script-payment Shelley address is a header byte plus a 28-byte hash"
        );
        // Byron base58 — still spendable, and still an input a Plutus
        // transaction can name.
        assert!(
            address_bytes("Ae2tdPwUPEZFRbyhz3cpfC2CumGzNkFBN2L42rcUc2yjQpEkxDbkPodpMAi").is_ok()
        );
        assert!(address_bytes("not-an-address").is_err());
    }

    #[test]
    fn the_application_order_differs_by_version_and_says_so() {
        // The strings are what a report prints, and the V3 arm applying only
        // the context is the difference a swap would erase.
        assert_eq!(
            Application::V1V2WithDatum.to_string(),
            "((script datum) redeemer) context"
        );
        assert_eq!(Application::V3ContextOnly.to_string(), "script context");
        assert_ne!(
            Application::V1V2WithoutDatum.to_string(),
            Application::V3ContextOnly.to_string()
        );
    }
}
