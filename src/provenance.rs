//! Per-requisite provenance: for each thing a script execution needs, whether
//! the transaction carries it or a state read has to answer for it.
//!
//! # What this module observes
//!
//! The raw on-chain CBOR of one transaction, decoded era-generically through
//! `pallas_traverse::MultiEraTx`, plus the outputs that transaction consumes
//! and refers to, fetched one leaf at a time. For each script execution it
//! reports, per requisite, `InTransaction` or `RequiresHydration` — and for
//! the second it **names the resolution path**.
//!
//! Every reading is a comparison of bytes against a hash the chain published:
//!
//! - a script is `InTransaction` when some script in the witness set hashes
//!   (`blake2b-224(language_tag ‖ bytes)`) to the hash the spent output's
//!   address names;
//! - a datum is `InTransaction` when some datum in the witness set hashes
//!   (`blake2b-256` over its *original* CBOR) to the hash the spent output
//!   carries.
//!
//! So "taken from the transaction" is never an assumption about an era; it is
//! a hash agreeing or not agreeing, per execution, on the bytes in hand.
//!
//! # Why this is a classifier and not an assertion
//!
//! CXP-ADA-2's deliverable 2 as written asks that every script execution's
//! script, datum, redeemer and inputs be taken *from the transaction*,
//! "asserted rather than assumed". The charter's own 2026-09-28 correction
//! measured that post-Babbage this is **false**: CIP-33 moves the script to
//! another UTxO and CIP-32 leaves the spent output's inline datum there too.
//! A function that asserted the deliverable's literal wording would therefore
//! have to fail on most of mainnet, and a function that assumed the opposite
//! would be wrong about the Alonzo era. So this module measures which it is,
//! per requisite, and the honest assertion is the one in
//! [`crate::prestate`]: that the strategy is `hydrated-from-node`.
//!
//! # What this module does NOT do
//!
//! - **It does not build a script context**, though one is now buildable.
//!   Reconstructing Plutus's `ScriptContext` from a settled transaction landed
//!   as CXP-ADA-1 deliverable 4 and lives in [`crate::oracle`]; this module
//!   classifies the script context as [`Carriage::RequiresHydration`] — the
//!   transaction does not carry one — and names that path as built. It does
//!   not call it, because this module's job is carriage and not execution.
//! - **It does not evaluate anything.** No CEK machine is started here and no
//!   budget is compared. What it does read off the bytes is the execution
//!   units the chain recorded for each redeemer, because those live in the
//!   witness set and are therefore an acquisition fact.
//! - **It does not consult Koios's `plutus_contracts` summary**, whose
//!   `bytecode` field was measured `null` for transactions whose script bytes
//!   are present in the CBOR.

use std::collections::BTreeSet;
use std::fmt;

use eyre::{eyre, Result};
use pallas_addresses::{Address, ShelleyPaymentPart};
use pallas_crypto::hash::Hasher;
use pallas_primitives::conway::RedeemerTag;
use pallas_traverse::{ComputeHash, MultiEraTx, OriginalHash};

use crate::chain::{ChainSource, UtxoRef, UtxoRow};

// ---------------------------------------------------------------------------
// Carriage
// ---------------------------------------------------------------------------

/// Where the bytes of one requisite actually are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Carriage {
    /// The bytes are inside the transaction, and a hash says so.
    InTransaction,
    /// The bytes are not in the transaction; a state read answers for them.
    RequiresHydration,
    /// The requisite does not exist for this execution.
    NotApplicable,
}

impl fmt::Display for Carriage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Carriage::InTransaction => write!(f, "in-transaction"),
            Carriage::RequiresHydration => write!(f, "requires-hydration"),
            Carriage::NotApplicable => write!(f, "not-applicable"),
        }
    }
}

/// One requisite of one script execution, classified.
#[derive(Debug, Clone)]
pub struct RequisiteReading {
    /// `script`, `datum`, `redeemer`, `inputs` or `script-context`.
    pub requisite: &'static str,
    pub carriage: Carriage,
    /// What was compared against what, so the reading can be re-run by hand.
    pub evidence: String,
    /// For `RequiresHydration`, the named path that answers it. `None` for
    /// `InTransaction`, because nothing has to be resolved.
    pub resolution_path: Option<String>,
    /// Whether that resolution path exists in this recorder today.
    pub resolution_built: bool,
}

/// One script execution the transaction performed.
#[derive(Debug, Clone)]
pub struct ScriptExecution {
    /// `spend`, `mint`, `cert`, `reward`, `vote` or `propose`.
    pub purpose: &'static str,
    /// The redeemer's index within its purpose.
    pub index: u32,
    /// The output this execution spends, for `spend`.
    pub spends: Option<UtxoRef>,
    /// The script hash the execution runs, when it is determinable from the
    /// bytes in hand. `None` for a purpose whose script is named by a policy
    /// id or a credential this module does not resolve.
    pub script_hash: Option<String>,
    /// Execution units the chain recorded, read out of the witness set.
    pub consensus_mem: u64,
    pub consensus_steps: u64,
    pub requisites: Vec<RequisiteReading>,
}

impl ScriptExecution {
    /// Whether every requisite of this execution travels with the transaction.
    pub fn is_self_contained(&self) -> bool {
        self.requisites
            .iter()
            .all(|r| r.carriage != Carriage::RequiresHydration)
    }
}

// ---------------------------------------------------------------------------
// Controls
// ---------------------------------------------------------------------------

/// The two-sided control on the byte search, so an `ABSENT` reading is
/// attributable to absence rather than to a search that cannot find anything.
#[derive(Debug, Clone)]
pub struct SearchControls {
    /// A needle that is not on the chain. Must be ABSENT.
    pub nonsense_needle: String,
    pub nonsense_found: bool,
    /// A needle the transaction structurally must contain — the first input's
    /// transaction id, which is in the body's input set by construction. Must
    /// be FOUND.
    pub structural_needle: String,
    pub structural_found: bool,
    /// The first executed script's hash, searched as a *reported* control.
    ///
    /// The charter's 2026-09-28 probe used this as its positive arm. CXP-ADA-2
    /// re-measured it and found it is **not structural**: a transaction that
    /// carries its script need not mention the script's hash anywhere, and the
    /// earliest mainnet Plutus execution does not. It is reported, not relied
    /// on.
    pub script_hash_needle: Option<String>,
    pub script_hash_found: bool,
}

impl SearchControls {
    /// Whether the instrument demonstrated it can report both presence and
    /// absence on this transaction.
    pub fn two_sided(&self) -> bool {
        !self.nonsense_found && self.structural_found
    }
}

// ---------------------------------------------------------------------------
// The reading for one transaction
// ---------------------------------------------------------------------------

/// Everything this module reads about one transaction.
#[derive(Debug, Clone)]
pub struct TxProvenance {
    pub tx_hash: String,
    /// The newest era format whose decoder accepted these bytes.
    ///
    /// **This is not the ledger era, and reporting it as one is wrong.**
    /// `MultiEraTx::decode` tries Conway first and returns on the first
    /// success, and the post-Alonzo transaction formats are compatible enough
    /// that a 2021 Alonzo transaction decodes cleanly as Conway. Measured
    /// 2026-10-31 on the earliest mainnet Plutus execution, which sits in
    /// epoch 290 (Alonzo) and decodes as `Conway`.
    ///
    /// So this field says "these bytes parse under that format", which is a
    /// fact about the encoding. The ledger era is a property of the
    /// transaction's EPOCH and is carried in [`Self::ledger_era`].
    pub decoded_as: String,
    /// The ledger era, from the epoch the transaction's block is in.
    ///
    /// `None` when the era was not supplied — which is the honest state for a
    /// transaction whose block is outside the captured window, because the
    /// era cannot be read off a bare transaction's bytes (see
    /// [`Self::decoded_as`]).
    pub ledger_era: Option<String>,
    /// `blake2b-256` over the body's original CBOR equalled the reported hash.
    pub body_hash_agrees: bool,
    /// `blake2b-256` over the WHOLE transaction CBOR equalled the reported
    /// hash. Must be false; see [`crate::fixture`].
    pub whole_cbor_hash_agrees: bool,
    pub cbor_len: usize,
    pub inputs: Vec<UtxoRef>,
    /// CIP-31 reference inputs: outputs the transaction reads without spending.
    pub reference_inputs: Vec<UtxoRef>,
    /// Scripts the witness set carries, by hash.
    pub witness_scripts: Vec<String>,
    /// Datums the witness set carries, by hash.
    pub witness_datums: Vec<String>,
    pub executions: Vec<ScriptExecution>,
    /// How many batched leaf reads were issued to resolve this transaction.
    pub hydration_rounds: u32,
    /// How many outputs those reads returned.
    pub leaves_resolved: usize,
    pub controls: SearchControls,
}

impl TxProvenance {
    /// Whether every requisite of every execution travels with the
    /// transaction. This is the `self-contained` claim, per transaction.
    pub fn is_self_contained(&self) -> bool {
        self.executions.iter().all(|e| e.is_self_contained())
    }

    /// Requisites that need a state read, counted.
    pub fn hydrated_requisite_count(&self) -> usize {
        self.executions
            .iter()
            .flat_map(|e| &e.requisites)
            .filter(|r| r.carriage == Carriage::RequiresHydration)
            .count()
    }
}

// ---------------------------------------------------------------------------
// Resolution paths, named
// ---------------------------------------------------------------------------

/// The named path that resolves a CIP-33 reference script.
pub const PATH_REFERENCE_SCRIPT: &str =
    "reference-input -> /utxo_info(_extended) -> reference_script.bytes, \
     verified blake2b-224(tag‖bytes) == the script hash the spent address names";

/// The named path that resolves a CIP-32 inline datum on a spent output.
pub const PATH_INLINE_DATUM: &str =
    "spent output-ref -> /utxo_info(_extended) -> inline_datum.bytes, \
     verified blake2b-256(bytes) == the output's datum_hash (answers with \
     is_spent=true, which is why a historic transaction resolves at all)";

/// The named path that resolves a datum the transaction references by hash
/// but does not carry.
pub const PATH_DATUM_BY_HASH: &str =
    "datum_hash -> /datum_info -> bytes, verified blake2b-256(bytes) == datum_hash";

/// The named path that assembles Plutus's `ScriptContext`.
pub const PATH_SCRIPT_CONTEXT: &str =
    "resolved inputs + reference inputs + body (fee, validity range, signatories, \
     mint, certs, withdrawals, redeemers) -> Plutus ScriptContext PlutusData, \
     via uplc TxInfoV1/V2/V3::from_transaction + into_script_context — BUILT in \
     crate::oracle (CXP-ADA-1 deliverable 4)";

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

fn tag_name(tag: RedeemerTag) -> &'static str {
    match tag {
        RedeemerTag::Spend => "spend",
        RedeemerTag::Mint => "mint",
        RedeemerTag::Cert => "cert",
        RedeemerTag::Reward => "reward",
        RedeemerTag::Vote => "vote",
        RedeemerTag::Propose => "propose",
    }
}

/// The script hash an address pays to, or `None` when the address pays to a
/// key rather than a script.
fn script_hash_of_address(bech32_or_raw: &str) -> Option<String> {
    let addr = Address::from_bech32(bech32_or_raw).ok()?;
    match addr {
        Address::Shelley(s) => match s.payment() {
            ShelleyPaymentPart::Script(h) => Some(h.to_string()),
            ShelleyPaymentPart::Key(_) => None,
        },
        _ => None,
    }
}

/// Classify one transaction's script executions from its raw CBOR.
///
/// `src` is consulted for the outputs this transaction consumes and refers to,
/// in **one** batched leaf read, and the round count is reported.
pub fn classify(
    tx_cbor: &[u8],
    expect_tx_hash: &str,
    src: &dyn ChainSource,
) -> Result<TxProvenance> {
    classify_in_era(tx_cbor, expect_tx_hash, src, None)
}

/// Classify one transaction, with the ledger era supplied when it is known.
///
/// `ledger_era` is NOT used to pick a decoder — the decoder probes, and every
/// post-Alonzo format decodes the others' transactions. It is carried through
/// so a report can state the era the chain was in rather than the format the
/// bytes happen to parse under, and `None` is reported as `unknown` rather
/// than quietly filled in from the probe.
pub fn classify_in_era(
    tx_cbor: &[u8],
    expect_tx_hash: &str,
    src: &dyn ChainSource,
    ledger_era: Option<&str>,
) -> Result<TxProvenance> {
    let tx = MultiEraTx::decode(tx_cbor)
        .map_err(|e| eyre!("transaction CBOR does not decode in any known era: {e}"))?;

    let body_hash = tx.hash().to_string();
    let whole_hash = Hasher::<256>::hash(tx_cbor).to_string();

    let inputs: Vec<UtxoRef> = tx
        .inputs_sorted_set()
        .iter()
        .map(|i| UtxoRef::new(i.hash().to_string(), i.index()))
        .collect();
    let reference_inputs: Vec<UtxoRef> = tx
        .reference_inputs()
        .iter()
        .map(|i| UtxoRef::new(i.hash().to_string(), i.index()))
        .collect();

    // Hash every script and datum the witness set carries, once. These are
    // the two sets a requisite is looked up in, and they are built from the
    // bytes rather than from any report about them.
    let mut witness_scripts: Vec<String> = Vec::new();
    for s in tx.plutus_v1_scripts() {
        witness_scripts.push(s.compute_hash().to_string());
    }
    for s in tx.plutus_v2_scripts() {
        witness_scripts.push(s.compute_hash().to_string());
    }
    for s in tx.plutus_v3_scripts() {
        witness_scripts.push(s.compute_hash().to_string());
    }
    let witness_datums: Vec<String> = tx
        .plutus_data()
        .iter()
        .map(|d| d.original_hash().to_string())
        .collect();

    // One batched leaf read for every output this transaction consumes or
    // refers to. This is the hydration round `hydrated-from-node` names.
    let mut wanted: Vec<UtxoRef> = inputs.clone();
    wanted.extend(reference_inputs.iter().cloned());
    let resolved: Vec<UtxoRow> = src.utxos(&wanted)?;
    let hydration_rounds = if wanted.is_empty() { 0 } else { 1 };

    let find = |r: &UtxoRef| resolved.iter().find(|row| &row.reference == r);

    let mut executions: Vec<ScriptExecution> = Vec::new();
    for red in tx.redeemers() {
        let purpose = tag_name(red.tag());
        let index = red.index();
        let units = red.ex_units();

        let spends = if red.tag() == RedeemerTag::Spend {
            inputs.get(index as usize).cloned()
        } else {
            None
        };

        let spent_row = spends.as_ref().and_then(find);
        let script_hash = spent_row.and_then(|row| script_hash_of_address(&row.address));

        let mut requisites: Vec<RequisiteReading> = Vec::new();

        // --- the script ---------------------------------------------------
        match &script_hash {
            Some(want) => {
                if witness_scripts.contains(want) {
                    requisites.push(RequisiteReading {
                        requisite: "script",
                        carriage: Carriage::InTransaction,
                        evidence: format!(
                            "a script in the witness set hashes to {want}, the hash the spent \
                             output's address names"
                        ),
                        resolution_path: None,
                        resolution_built: true,
                    });
                } else {
                    // Which reference input carries it? Name it when the leaf
                    // read answered, so the path is concrete rather than
                    // generic.
                    let carrier = reference_inputs.iter().find(|r| {
                        find(r)
                            .and_then(|row| row.reference_script.as_ref())
                            .map(|s| s.script_hash == *want)
                            .unwrap_or(false)
                    });
                    requisites.push(RequisiteReading {
                        requisite: "script",
                        carriage: Carriage::RequiresHydration,
                        evidence: match carrier {
                            Some(r) => format!(
                                "no script in the witness set hashes to {want}; the reference \
                                 input {r} carries it (CIP-33)"
                            ),
                            None => format!(
                                "no script in the witness set hashes to {want}, and no resolved \
                                 reference input carries it either — the script is reachable \
                                 only by a further state read"
                            ),
                        },
                        resolution_path: Some(PATH_REFERENCE_SCRIPT.to_string()),
                        resolution_built: true,
                    });
                }
            }
            None => requisites.push(RequisiteReading {
                requisite: "script",
                carriage: Carriage::RequiresHydration,
                evidence: format!(
                    "the script this `{purpose}` redeemer runs is not named by a spent output's \
                     address, so which script it is cannot be read off the transaction"
                ),
                resolution_path: Some(
                    "purpose-specific: a mint's policy id, a certificate's or a vote's script \
                     credential — NOT BUILT for this purpose"
                        .to_string(),
                ),
                resolution_built: false,
            }),
        }

        // --- the datum ----------------------------------------------------
        match spent_row {
            Some(row) => match (&row.datum_hash, &row.inline_datum) {
                (None, None) => requisites.push(RequisiteReading {
                    requisite: "datum",
                    carriage: Carriage::NotApplicable,
                    evidence: "the spent output carries neither a datum nor a datum hash"
                        .to_string(),
                    resolution_path: None,
                    resolution_built: true,
                }),
                (Some(dh), inline) => {
                    if witness_datums.contains(dh) {
                        requisites.push(RequisiteReading {
                            requisite: "datum",
                            carriage: Carriage::InTransaction,
                            evidence: format!(
                                "a datum in the witness set hashes to {dh}, the spent output's \
                                 datum_hash"
                            ),
                            resolution_path: None,
                            resolution_built: true,
                        });
                    } else {
                        let (path, how) = if inline.is_some() {
                            (
                                PATH_INLINE_DATUM,
                                "an inline datum (CIP-32) on the spent output",
                            )
                        } else {
                            (PATH_DATUM_BY_HASH, "a datum named only by its hash")
                        };
                        requisites.push(RequisiteReading {
                            requisite: "datum",
                            carriage: Carriage::RequiresHydration,
                            evidence: format!(
                                "no datum in the witness set hashes to {dh}; it is {how}"
                            ),
                            resolution_path: Some(path.to_string()),
                            resolution_built: true,
                        });
                    }
                }
                (None, Some(_)) => requisites.push(RequisiteReading {
                    requisite: "datum",
                    carriage: Carriage::RequiresHydration,
                    evidence: "the spent output carries an inline datum but the source reported \
                               no datum_hash for it, so its identity cannot be checked"
                        .to_string(),
                    resolution_path: Some(PATH_INLINE_DATUM.to_string()),
                    resolution_built: false,
                }),
            },
            None => requisites.push(RequisiteReading {
                requisite: "datum",
                carriage: Carriage::RequiresHydration,
                evidence: match &spends {
                    Some(r) => format!(
                        "the spent output {r} was not resolved, so whether it \
                                        carries a datum is unknown"
                    ),
                    None => format!(
                        "a `{purpose}` redeemer spends no output, so no datum is \
                                     named by one"
                    ),
                },
                resolution_path: Some(PATH_INLINE_DATUM.to_string()),
                resolution_built: true,
            }),
        }

        // --- the redeemer -------------------------------------------------
        //
        // The redeemer IS the witness-set entry being read, so this is
        // `InTransaction` by construction. It is listed anyway, with the
        // byte length, because a requisite list that silently omitted the one
        // requisite that always travels would make the others look worse than
        // they are.
        requisites.push(RequisiteReading {
            requisite: "redeemer",
            carriage: Carriage::InTransaction,
            evidence: format!(
                "the `{purpose}` redeemer at index {index} is a witness-set entry of this \
                 transaction, carrying ex_units mem={} steps={}",
                units.mem, units.steps
            ),
            resolution_path: None,
            resolution_built: true,
        });

        // --- the inputs ---------------------------------------------------
        //
        // The transaction names its inputs; it does not carry their contents.
        // On a UTXO chain that distinction is the whole of §1.4's question.
        requisites.push(RequisiteReading {
            requisite: "inputs",
            carriage: Carriage::RequiresHydration,
            evidence: format!(
                "the body names {} input(s) and {} reference input(s) by output-ref; their \
                 addresses, values, datums and scripts are not in the transaction",
                inputs.len(),
                reference_inputs.len()
            ),
            resolution_path: Some(PATH_INLINE_DATUM.to_string()),
            resolution_built: true,
        });

        // --- the script context -------------------------------------------
        requisites.push(RequisiteReading {
            requisite: "script-context",
            carriage: Carriage::RequiresHydration,
            evidence: "Plutus applies a ScriptContext the transaction does not carry; it is \
                       assembled from the body plus every resolved input"
                .to_string(),
            resolution_path: Some(PATH_SCRIPT_CONTEXT.to_string()),
            // Built as of CXP-ADA-1 deliverable 4. The carriage stays
            // `RequiresHydration` and that is not a contradiction: the
            // transaction still does not CARRY a script context, which is what
            // this column reports. What changed is that the path which answers
            // for it now exists ([`crate::oracle`]) instead of being named.
            resolution_built: true,
        });

        executions.push(ScriptExecution {
            purpose,
            index,
            spends,
            script_hash,
            consensus_mem: units.mem,
            consensus_steps: units.steps,
            requisites,
        });
    }

    // --- the two-sided control on the byte search -------------------------
    let hex_cbor = hex::encode(tx_cbor);
    let nonsense = "deadbeefcafebabe0123456789abcdeffedcba9876543210deadbeefcafe".to_string();
    let structural = inputs
        .first()
        .map(|i| i.tx_hash.clone())
        .unwrap_or_default();
    let script_needle = executions.iter().find_map(|e| e.script_hash.clone());

    let controls = SearchControls {
        nonsense_found: hex_cbor.contains(&nonsense),
        nonsense_needle: nonsense,
        structural_found: !structural.is_empty() && hex_cbor.contains(&structural),
        structural_needle: structural,
        script_hash_found: script_needle
            .as_ref()
            .map(|h| hex_cbor.contains(h))
            .unwrap_or(false),
        script_hash_needle: script_needle,
    };

    Ok(TxProvenance {
        tx_hash: expect_tx_hash.to_string(),
        decoded_as: format!("{:?}", tx.era()),
        ledger_era: ledger_era.map(|s| s.to_string()),
        body_hash_agrees: body_hash == expect_tx_hash,
        whole_cbor_hash_agrees: whole_hash == expect_tx_hash,
        cbor_len: tx_cbor.len(),
        inputs,
        reference_inputs,
        witness_scripts,
        witness_datums,
        executions,
        hydration_rounds,
        leaves_resolved: resolved.len(),
        controls,
    })
}

/// A human-readable rendering of one transaction's provenance.
pub fn render(p: &TxProvenance) -> String {
    let mut s = String::new();
    s.push_str(&format!("transaction {}\n", p.tx_hash));
    s.push_str(&format!(
        "  ledger era {}   decoded as {} (the newest format that parses these bytes, NOT the \
         era)\n  cbor {} bytes   {} input(s), {} reference input(s)\n",
        p.ledger_era.as_deref().unwrap_or("UNKNOWN (not supplied)"),
        p.decoded_as,
        p.cbor_len,
        p.inputs.len(),
        p.reference_inputs.len()
    ));
    s.push_str(&format!(
        "  identity: blake2b-256(body cbor) == tx_hash: {}; blake2b-256(whole cbor) == tx_hash: \
         {} (must be false)\n",
        if p.body_hash_agrees {
            "AGREES"
        } else {
            "DISAGREES"
        },
        if p.whole_cbor_hash_agrees {
            "AGREES"
        } else {
            "disagrees"
        }
    ));
    s.push_str(&format!(
        "  witness set carries {} plutus script(s) and {} datum(s)\n",
        p.witness_scripts.len(),
        p.witness_datums.len()
    ));
    s.push_str(&format!(
        "  hydration: {} round(s), {} leaf/leaves resolved\n",
        p.hydration_rounds, p.leaves_resolved
    ));
    s.push_str("  search controls (two-sided):\n");
    s.push_str(&format!(
        "    nonsense needle          {}  (must be ABSENT)\n",
        if p.controls.nonsense_found {
            "FOUND"
        } else {
            "ABSENT"
        }
    ));
    s.push_str(&format!(
        "    structural needle        {}  (first input's tx id; must be FOUND)\n",
        if p.controls.structural_found {
            "FOUND"
        } else {
            "ABSENT"
        }
    ));
    match &p.controls.script_hash_needle {
        Some(h) => s.push_str(&format!(
            "    script hash {}  {}  (reported, NOT structural)\n",
            &h[..8.min(h.len())],
            if p.controls.script_hash_found {
                "FOUND "
            } else {
                "ABSENT"
            }
        )),
        None => s.push_str("    script hash              n/a   (no execution named one)\n"),
    }
    s.push_str(&format!(
        "    => instrument is two-sided: {}\n",
        if p.controls.two_sided() { "YES" } else { "NO" }
    ));

    for e in &p.executions {
        s.push_str(&format!(
            "  execution {}[{}]  script {}  consensus mem={} steps={}\n",
            e.purpose,
            e.index,
            e.script_hash.as_deref().unwrap_or("<undetermined>"),
            e.consensus_mem,
            e.consensus_steps
        ));
        if let Some(r) = &e.spends {
            s.push_str(&format!("    spends {r}\n"));
        }
        for r in &e.requisites {
            s.push_str(&format!(
                "    {:<15} {:<19} {}\n",
                r.requisite,
                r.carriage.to_string(),
                r.evidence
            ));
            if let Some(path) = &r.resolution_path {
                s.push_str(&format!(
                    "    {:<15} path{}: {}\n",
                    "",
                    if r.resolution_built { "" } else { " (UNBUILT)" },
                    path
                ));
            }
        }
    }
    s.push_str(&format!(
        "  => this transaction is self-contained: {}  ({} requisite(s) require hydration)\n",
        if p.is_self_contained() { "YES" } else { "NO" },
        p.hydrated_requisite_count()
    ));
    s
}

/// The reference inputs and reference scripts of one transaction, as
/// deliverable 3 asks for them: enumerated, with the resolution path named.
#[derive(Debug, Clone)]
pub struct ReferenceReading {
    pub reference: UtxoRef,
    pub resolved: bool,
    /// `true` when the referred output was already consumed. A historic
    /// transaction's references usually are, which is why the path only works
    /// because `/utxo_info` serves spent outputs.
    pub is_spent: bool,
    pub carries_script: Option<String>,
    pub script_type: Option<String>,
    pub carries_inline_datum: bool,
    pub resolution_path: &'static str,
}

/// Enumerate a transaction's reference inputs and what each one carries.
pub fn reference_inputs(p: &TxProvenance, src: &dyn ChainSource) -> Result<Vec<ReferenceReading>> {
    if p.reference_inputs.is_empty() {
        return Ok(Vec::new());
    }
    let rows = src.utxos(&p.reference_inputs)?;
    let mut out = Vec::new();
    for r in &p.reference_inputs {
        let row = rows.iter().find(|x| &x.reference == r);
        out.push(ReferenceReading {
            reference: r.clone(),
            resolved: row.is_some(),
            is_spent: row.map(|x| x.is_spent).unwrap_or(false),
            carries_script: row
                .and_then(|x| x.reference_script.as_ref())
                .map(|s| s.script_hash.clone()),
            script_type: row
                .and_then(|x| x.reference_script.as_ref())
                .map(|s| s.script_type.clone()),
            carries_inline_datum: row.map(|x| x.inline_datum.is_some()).unwrap_or(false),
            resolution_path: PATH_REFERENCE_SCRIPT,
        });
    }
    Ok(out)
}

/// The distinct script hashes a transaction's executions need.
pub fn needed_script_hashes(p: &TxProvenance) -> BTreeSet<String> {
    p.executions
        .iter()
        .filter_map(|e| e.script_hash.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_address_yields_its_script_hash() {
        // The enterprise script address of the PlutusV3 validator the
        // embedded-script case spends; its payment part is the script hash.
        let addr = "addr1w8z0xlftcx54tn7uxdvhk0qgj9u7hmlaccjthnc9kvu4pmcyemglm";
        assert_eq!(
            script_hash_of_address(addr).as_deref(),
            Some("c4f37d2bc1a955cfdc33597b3c089179ebeffdc624bbcf05b33950ef")
        );
    }

    #[test]
    fn a_key_address_yields_no_script_hash() {
        // A payment-key address names no script, and returning one anyway
        // would make every requisite look resolvable.
        let addr = "addr1q9d34spgg2kdy5hg4ej3h35m3lzroasyt2hq4nz6kg9anlg2a8n4m\
                    7q9d9z6p6kq4m0h2d8k4kqd3t0lgt0e8rk4rvsq0xmdy";
        assert_eq!(script_hash_of_address(addr), None);
    }

    #[test]
    fn an_undecodable_transaction_is_refused_rather_than_classified() {
        struct Nothing;
        impl ChainSource for Nothing {
            fn origin(&self) -> crate::chain::ChainOrigin {
                crate::chain::ChainOrigin::Fixture
            }
            fn blocks_in_range(&self, _: u64, _: u64) -> Result<Vec<crate::chain::BlockRow>> {
                Ok(vec![])
            }
            fn block_txs(&self, _: &[String]) -> Result<Vec<crate::chain::BlockTxRow>> {
                Ok(vec![])
            }
            fn tx_cbor(&self, _: &str) -> Result<Vec<u8>> {
                Ok(vec![])
            }
            fn scripts(&self, _: &[String]) -> Result<Vec<crate::chain::ScriptRow>> {
                Ok(vec![])
            }
            fn utxos(&self, _: &[UtxoRef]) -> Result<Vec<UtxoRow>> {
                Ok(vec![])
            }
            fn datums(&self, _: &[String]) -> Result<Vec<crate::chain::DatumRow>> {
                Ok(vec![])
            }
            fn tx_epoch(&self, _: &str) -> Result<u64> {
                Err(eyre!("not captured"))
            }
            fn epoch_params(&self, _: u64) -> Result<crate::chain::EpochParams> {
                Err(eyre!("not captured"))
            }
            fn protocol_versions(&self) -> Result<Vec<crate::chain::ProtocolVersionRow>> {
                Ok(vec![])
            }
        }
        let err = classify(&[0xff, 0xff, 0xff], "nope", &Nothing)
            .expect_err("garbage must not classify as an empty transaction");
        assert!(err.to_string().contains("does not decode"), "got: {err}");
    }
}
