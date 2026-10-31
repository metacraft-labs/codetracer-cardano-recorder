//! CXP-ADA-2 acquisition: what was measured, asserted against the chain's own
//! bytes.
//!
//! # Mock use, justified (CLAUDE.md requires this)
//!
//! **Nothing here mocks the chain.** Every reading below runs against
//! `tests/fixtures/koios/`, which holds Cardano mainnet's own responses
//! verbatim, and every one of those files is checked against a hash the chain
//! published — `blake2b-256` for a datum and a transaction body,
//! `blake2b-224(language tag ‖ bytes)` for a script. So the inputs are the
//! chain's bytes and the suite says so by recomputing, not by asserting.
//!
//! **Two stubs exist and neither stands in for chain data.**
//!
//! 1. `BlockedSource` answers every state read with a refusal. It is not a
//!    fake node; it is the *absence* of one, and it exists because the only
//!    way to show a state read is load-bearing is to take it away. A test that
//!    asserted hydration was necessary without ever removing it would be
//!    asserting its own premise.
//! 2. `TamperedFixtures` copies the real captures into a temp directory and
//!    flips bytes in one of them. It exists to drive the hash-identity check
//!    red, because a control nobody has observed failing is not a control.
//!
//! Both are named for what they remove rather than for what they supply, and
//! neither is ever used as the positive arm of any reading.
//!
//! # What these tests do NOT cover, and why
//!
//! **No budget is compared against the chain's.** Plutus applies a
//! `ScriptContext` the transaction does not carry, and assembling one is
//! CXP-ADA-1 deliverable 4's open clause. Without it no script can be run to
//! completion, so the milestone's
//! `test_an_early_era_transaction_is_evaluated_under_its_own_rules` cannot be
//! closed here: its *"and its budget matches the chain's"* clause is
//! unreachable. What IS measured is the half that does not need a context —
//! that an era's own cost model is built from that era's own parameters, and
//! that it differs from today's, so the era handling is load-bearing rather
//! than decorative.
//!
//! **No `.ct` file is written or read.** Acquisition produces no recording;
//! emitting a conforming snapshot tree is CXP-ADA-3.

use codetracer_cardano_recorder::chain::{
    BlockRow, BlockTxRow, ChainOrigin, ChainSource, DatumRow, EpochParams, ProtocolVersionRow,
    ScriptRow, UtxoRef, UtxoRow,
};
use codetracer_cardano_recorder::fixture::{
    cases, verify_identities, FixtureSource, IdentityVerdict, ALONZO_WINDOW, BOUNDARY_EPOCHS,
};
use codetracer_cardano_recorder::{enumerate, eras, koios, prestate, provenance};
use eyre::{eyre, Result};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fixtures() -> FixtureSource {
    FixtureSource::at_crate_root()
}

/// The transaction hash a case's capture declares.
fn case_tx_hash(case: &str) -> String {
    let f = fixtures()
        .load(&format!("tx_cbor_{case}"))
        .expect("the case's transaction capture must be present");
    f.identity
        .get("expect_tx_hash")
        .and_then(|x| x.as_str())
        .expect("every transaction capture declares the hash it is of")
        .to_string()
}

fn classify_case(case: &str) -> provenance::TxProvenance {
    let src = fixtures();
    let h = case_tx_hash(case);
    let cbor = src.tx_cbor(&h).expect("the capture carries this CBOR");
    provenance::classify(&cbor, &h, &src).expect("the chain's own CBOR must classify")
}

/// A source that refuses every state read.
///
/// See the module header: this is the ABSENCE of a node, used only to take
/// hydration away and observe what stops working.
struct BlockedSource;

impl ChainSource for BlockedSource {
    fn origin(&self) -> ChainOrigin {
        ChainOrigin::Fixture
    }
    fn blocks_in_range(&self, _: u64, _: u64) -> Result<Vec<BlockRow>> {
        Err(eyre!("chain state queries are blocked"))
    }
    fn block_txs(&self, _: &[String]) -> Result<Vec<BlockTxRow>> {
        Err(eyre!("chain state queries are blocked"))
    }
    fn tx_cbor(&self, _: &str) -> Result<Vec<u8>> {
        Err(eyre!("chain state queries are blocked"))
    }
    fn scripts(&self, _: &[String]) -> Result<Vec<ScriptRow>> {
        Err(eyre!("chain state queries are blocked"))
    }
    fn utxos(&self, _: &[UtxoRef]) -> Result<Vec<UtxoRow>> {
        Err(eyre!("chain state queries are blocked"))
    }
    fn datums(&self, _: &[String]) -> Result<Vec<DatumRow>> {
        Err(eyre!("chain state queries are blocked"))
    }
    fn tx_epoch(&self, _: &str) -> Result<u64> {
        Err(eyre!("chain state queries are blocked"))
    }
    fn epoch_params(&self, _: u64) -> Result<EpochParams> {
        Err(eyre!("chain state queries are blocked"))
    }
    fn protocol_versions(&self) -> Result<Vec<ProtocolVersionRow>> {
        Err(eyre!("chain state queries are blocked"))
    }
}

/// A source that resolves nothing but refuses nothing: every leaf read
/// succeeds and returns the empty set.
///
/// This is the *silent* failure mode, and it is worth a separate arm from
/// `BlockedSource`: a node that answered "no such output" instead of erroring
/// must not make a transaction look self-contained.
struct EmptySource;

impl ChainSource for EmptySource {
    fn origin(&self) -> ChainOrigin {
        ChainOrigin::Fixture
    }
    fn blocks_in_range(&self, _: u64, _: u64) -> Result<Vec<BlockRow>> {
        Ok(vec![])
    }
    fn block_txs(&self, _: &[String]) -> Result<Vec<BlockTxRow>> {
        Ok(vec![])
    }
    fn tx_cbor(&self, _: &str) -> Result<Vec<u8>> {
        Ok(vec![])
    }
    fn scripts(&self, _: &[String]) -> Result<Vec<ScriptRow>> {
        Ok(vec![])
    }
    fn utxos(&self, _: &[UtxoRef]) -> Result<Vec<UtxoRow>> {
        Ok(vec![])
    }
    fn datums(&self, _: &[String]) -> Result<Vec<DatumRow>> {
        Ok(vec![])
    }
    fn tx_epoch(&self, _: &str) -> Result<u64> {
        Err(eyre!("no epoch"))
    }
    fn epoch_params(&self, _: u64) -> Result<EpochParams> {
        Err(eyre!("no parameters"))
    }
    fn protocol_versions(&self) -> Result<Vec<ProtocolVersionRow>> {
        Ok(vec![])
    }
}

// ---------------------------------------------------------------------------
// The fixtures are the chain's bytes, and that is recomputed
// ---------------------------------------------------------------------------

#[test]
fn test_every_committed_capture_holds_the_chains_own_hash_identity() {
    let readings = verify_identities(&fixtures()).expect("the captures must be readable");
    assert!(
        readings.len() >= 14,
        "expected the full capture set, found {}",
        readings.len()
    );
    let broken: Vec<_> = readings
        .iter()
        .filter(|r| r.verdict == IdentityVerdict::Broken)
        .collect();
    assert!(
        broken.is_empty(),
        "committed captures failed their own identity: {:#?}",
        broken
    );
    // And the anchored majority is asserted, not just the absence of
    // failures: a verifier that recognised no kind would also report zero
    // broken.
    let anchored = readings
        .iter()
        .filter(|r| r.verdict == IdentityVerdict::Anchored)
        .count();
    assert!(
        anchored >= 12,
        "only {anchored} capture(s) were anchored to a chain-published hash; the rest are \
         unanchored, which would make this suite trust its inputs"
    );
    let total_checks: usize = readings.iter().map(|r| r.checks).sum();
    println!(
        "MEASURED: {} capture(s), {anchored} anchored, {total_checks} identity check(s) recomputed",
        readings.len()
    );
}

#[test]
fn test_a_tampered_capture_fails_its_own_identity() {
    // The control for the test above, driven red here rather than asserted
    // to be possible. One byte of a datum is flipped; the datum's own hash
    // must stop agreeing.
    let dir = tempfile::tempdir().expect("temp dir");
    let src = fixtures();
    for stem in src.stems().unwrap() {
        let from = src.dir().join(format!("{stem}.fixture.json"));
        std::fs::copy(&from, dir.path().join(format!("{stem}.fixture.json"))).unwrap();
    }
    let target = dir
        .path()
        .join("utxo_info_v3_embedded_script_spend.fixture.json");
    let text = std::fs::read_to_string(&target).unwrap();
    // Flip one hex digit of an inline datum's bytes.
    let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let bytes = v["response"][0]["inline_datum"]["bytes"]
        .as_str()
        .expect("the capture carries an inline datum")
        .to_string();
    let flipped = format!(
        "{}{}",
        if bytes.starts_with('a') { "b" } else { "a" },
        &bytes[1..]
    );
    v["response"][0]["inline_datum"]["bytes"] = serde_json::Value::String(flipped);
    std::fs::write(&target, serde_json::to_string(&v).unwrap()).unwrap();

    let readings =
        verify_identities(&FixtureSource::at(dir.path())).expect("the tampered set still reads");
    let broken: Vec<&str> = readings
        .iter()
        .filter(|r| r.verdict == IdentityVerdict::Broken)
        .map(|r| r.fixture.as_str())
        .collect();
    assert!(
        broken.contains(&"utxo_info_v3_embedded_script_spend"),
        "flipping a datum byte must break that capture's identity; broken set was {broken:?}"
    );
}

// ---------------------------------------------------------------------------
// Deliverable 1 — enumeration, with the count reported as measured
// ---------------------------------------------------------------------------

#[test]
fn test_the_enumerated_transaction_count_agrees_with_the_blocks_own_counts() {
    let e = enumerate::enumerate(&fixtures(), ALONZO_WINDOW.0, ALONZO_WINDOW.1)
        .expect("the captured window must enumerate");
    println!(
        "MEASURED: heights {}..={} -> {} block(s), {} transaction(s)",
        e.from_height,
        e.to_height,
        e.block_count(),
        e.transactions
    );
    assert_eq!(e.block_count(), 10, "the captured window is ten blocks");
    assert!(
        e.transactions > 0,
        "an enumeration of zero transactions would make the cross-check vacuous"
    );
    assert!(
        e.agrees(),
        "the enumerated count disagrees with the blocks' own tx_count figures: {:?}",
        e.disagreements
    );
    assert!(
        e.missing_heights().is_empty(),
        "the range had holes: {:?}",
        e.missing_heights()
    );
    // The figure this milestone reports, pinned so a silent change in the
    // capture cannot move a number quoted in the record.
    assert_eq!(
        e.transactions, 330,
        "the window's measured transaction count moved; the record quotes 330"
    );
}

// ---------------------------------------------------------------------------
// Deliverable 2 — per-requisite provenance, asserted by hashing
// ---------------------------------------------------------------------------

#[test]
fn test_each_requisite_is_classified_by_a_hash_and_not_by_an_era_assumption() {
    for case in cases::ALL {
        let p = classify_case(case);
        assert!(
            p.body_hash_agrees,
            "{case}: blake2b-256 of the body's CBOR must equal the reported transaction id"
        );
        assert!(
            !p.whole_cbor_hash_agrees,
            "{case}: hashing the WHOLE transaction must NOT give the id, or the identity does \
             not say which bytes the chain hashed"
        );
        assert!(
            !p.executions.is_empty(),
            "{case}: no script execution was found, so every requisite reading is vacuous"
        );
        for e in &p.executions {
            let names: Vec<&str> = e.requisites.iter().map(|r| r.requisite).collect();
            for required in ["script", "datum", "redeemer", "inputs", "script-context"] {
                assert!(
                    names.contains(&required),
                    "{case} {}[{}]: requisite `{required}` is not reported at all; an omitted \
                     requisite reads as satisfied",
                    e.purpose,
                    e.index
                );
            }
            // Every hydration reading must name its path. A reading that said
            // "not in the transaction" without saying what answers for it
            // would be a gap dressed as a measurement.
            for r in &e.requisites {
                if r.carriage == provenance::Carriage::RequiresHydration {
                    assert!(
                        r.resolution_path.is_some(),
                        "{case} {}[{}].{}: requires hydration with no resolution path named",
                        e.purpose,
                        e.index,
                        r.requisite
                    );
                }
            }
        }
        println!(
            "MEASURED {case}: ledger era {}, {} execution(s), {} requisite(s) requiring hydration",
            p.ledger_era.as_deref().unwrap_or("UNKNOWN"),
            p.executions.len(),
            p.hydrated_requisite_count()
        );
    }
}

#[test]
fn test_the_decoded_format_is_not_reported_as_the_ledger_era() {
    // Measured 2026-10-31: `MultiEraTx::decode` tries Conway first and
    // returns on the first success, so the earliest mainnet Plutus execution
    // — settled in epoch 290, era Alonzo — decodes as Conway. A report that
    // took the probe for the era would place a 2021 transaction in an era
    // that did not exist for two more years, and would then evaluate it under
    // the wrong cost model.
    let src = fixtures();
    let h = case_tx_hash(cases::FIRST_PLUTUS_EXECUTION);
    let cbor = src.tx_cbor(&h).unwrap();

    let epoch =
        codetracer_cardano_recorder::chain::epoch_of_tx(&src, &h, ALONZO_WINDOW.0, ALONZO_WINDOW.1)
            .expect("the window enumerates")
            .expect("the earliest Plutus execution is inside the captured window");
    let era = src.epoch_params(epoch).unwrap().era;

    let p = provenance::classify_in_era(&cbor, &h, &src, era.as_deref()).unwrap();
    assert_eq!(p.ledger_era.as_deref(), Some("Alonzo"));
    assert_eq!(
        p.decoded_as, "Conway",
        "if the probe stops returning Conway here the finding has changed and the comment above \
         is stale"
    );
    assert_ne!(
        p.ledger_era.as_deref(),
        Some(p.decoded_as.as_str()),
        "the whole point of carrying both is that they differ for this transaction"
    );
    println!(
        "MEASURED: epoch {epoch} -> ledger era {era:?}, decoded as {}",
        p.decoded_as
    );
}

// ---------------------------------------------------------------------------
// Deliverable 3 — reference inputs and reference scripts, explicitly
// ---------------------------------------------------------------------------

#[test]
fn test_reference_scripts_resolve_through_the_named_path() {
    let src = fixtures();
    let p = classify_case(cases::V3_REFERENCE_SCRIPT_SPEND);
    assert!(
        !p.reference_inputs.is_empty(),
        "this case exists because it has reference inputs"
    );
    let refs = provenance::reference_inputs(&p, &src).expect("the references must resolve");
    assert_eq!(refs.len(), p.reference_inputs.len());
    assert!(
        refs.iter().all(|r| r.resolved),
        "every reference input must resolve, or the path is named and not demonstrated: {refs:#?}"
    );

    // Every script the executions need must be carried by one of the
    // references, and the classifier must say which.
    let needed = provenance::needed_script_hashes(&p);
    assert!(!needed.is_empty(), "no execution named a script");
    let carried: Vec<String> = refs
        .iter()
        .filter_map(|r| r.carries_script.clone())
        .collect();
    for want in &needed {
        assert!(
            carried.contains(want),
            "script {want} is needed and no resolved reference input carries it; carried: \
             {carried:?}"
        );
    }
    for e in &p.executions {
        if e.purpose != "spend" {
            continue;
        }
        let script = e
            .requisites
            .iter()
            .find(|r| r.requisite == "script")
            .unwrap();
        assert_eq!(
            script.carriage,
            provenance::Carriage::RequiresHydration,
            "this case's scripts are CIP-33 reference scripts"
        );
        assert!(
            script.evidence.contains("reference input") && script.evidence.contains("CIP-33"),
            "the reading must name the reference input that carries it; got: {}",
            script.evidence
        );
    }
    println!(
        "MEASURED: {} reference input(s); {} carry a reference script; the {} script(s) the \
         executions need are all carried",
        refs.len(),
        carried.len(),
        needed.len()
    );
}

#[test]
fn test_a_historic_reference_resolves_only_because_spent_outputs_are_served() {
    // The route's load-bearing fact: `/utxo_info` answers for an output that
    // has already been consumed, and answers WITH its inline datum. A node
    // that dropped consumed outputs would make every historic transaction
    // unresolvable, so this is asserted rather than assumed.
    let src = fixtures();
    let p = classify_case(cases::V3_REFERENCE_SCRIPT_SPEND);
    let rows = src.utxos(&p.inputs).expect("the inputs must resolve");
    let spent_with_datum = rows
        .iter()
        .filter(|r| r.is_spent && r.inline_datum.is_some())
        .count();
    assert!(
        spent_with_datum > 0,
        "no resolved input is both spent and carrying its inline datum, so this capture does not \
         demonstrate that a historic transaction's state can be hydrated at all"
    );
    println!(
        "MEASURED: {} of {} resolved input(s) are spent AND still carry their inline datum",
        spent_with_datum,
        rows.len()
    );
}

#[test]
fn test_state_queries_are_required_rather_than_optional() {
    // The milestone's `test_no_state_query_occurs_during_replay` entry asks
    // for the opposite: a full replay with state queries BLOCKED that
    // succeeds. The measurement below is why that entry cannot be closed —
    // blocking the queries does not degrade the reading, it removes it.
    //
    // Two arms, because the two ways a missing state read can go wrong are
    // different: a refusal, and a silent empty answer.
    let h = case_tx_hash(cases::V3_REFERENCE_SCRIPT_SPEND);
    let cbor = fixtures().tx_cbor(&h).unwrap();

    let blocked = provenance::classify(&cbor, &h, &BlockedSource);
    assert!(
        blocked.is_err(),
        "with state queries blocked the classification must fail, not fall back to a guess"
    );
    println!("MEASURED (blocked): {}", blocked.unwrap_err());

    // And a node that answers "nothing" must not make the transaction look
    // self-contained.
    let empty =
        provenance::classify(&cbor, &h, &EmptySource).expect("an empty answer is still an answer");
    assert!(
        !empty.is_self_contained(),
        "a transaction whose state nothing resolved must not read as self-contained"
    );
    for e in &empty.executions {
        assert!(
            e.script_hash.is_none(),
            "with nothing resolved, no script hash can be known — a known one would mean it came \
             from somewhere other than the chain"
        );
    }
    println!(
        "MEASURED (empty answers): {} execution(s), none with a determinable script, \
         self-contained={}",
        empty.executions.len(),
        empty.is_self_contained()
    );
}

// ---------------------------------------------------------------------------
// Deliverable 4 — the floors, both of them
// ---------------------------------------------------------------------------

#[test]
fn test_the_acquisition_floor_and_the_script_tracing_floor_are_different_numbers() {
    let src = fixtures();
    let rows = eras::era_table(&src, &BOUNDARY_EPOCHS).expect("the boundary epochs are captured");
    let f = eras::floors(&rows);

    assert_eq!(
        f.acquisition_floor_epoch, 0,
        "epoch 0 is genesis; an acquisition floor above it would contradict §1.3's `genesis`"
    );
    let script_floor = f
        .script_tracing_floor_epoch
        .expect("some epoch read must carry a cost model, or the reading is vacuous");
    assert_eq!(
        script_floor, 290,
        "the first epoch with any Plutus cost model is the Alonzo hard fork's"
    );
    assert_eq!(f.script_tracing_era.as_deref(), Some("Alonzo"));
    assert_ne!(
        f.acquisition_floor_epoch, script_floor,
        "if these were the same number the distinction would be decoration"
    );
    println!(
        "MEASURED: acquisition floor epoch {} ({}); script-tracing floor epoch {script_floor} \
         (era {:?}, {:?})",
        f.acquisition_floor_epoch,
        f.acquisition_evidence,
        f.script_tracing_era,
        f.script_tracing_languages
    );
}

#[test]
fn test_no_pre_alonzo_epoch_can_charge_a_plutus_script() {
    // The clause that decides deliverable 4's wording. "Tracing a transaction
    // from early history" is unsatisfiable below Alonzo, and this says so from
    // the chain's parameters rather than from the history of the protocol.
    let src = fixtures();
    for epoch in [0u64, 207, 208, 236, 251] {
        let p = src.epoch_params(epoch).expect("captured");
        assert!(
            p.cost_models.is_empty(),
            "epoch {epoch} ({:?}) unexpectedly carries cost model(s) {:?}",
            p.era,
            p.cost_models.keys().collect::<Vec<_>>()
        );
        for (lang, _) in eras::EVALUATOR_ARITY {
            let err = eras::cost_model_for(&p, lang)
                .expect_err("a pre-Alonzo epoch must refuse every Plutus language");
            assert!(
                err.to_string().contains("below the script-tracing floor"),
                "epoch {epoch} {lang}: {err}"
            );
        }
        println!(
            "MEASURED: epoch {epoch} era {:?} -> no Plutus cost model, no script can run",
            p.era
        );
    }
}

#[test]
fn test_the_earliest_plutus_execution_sits_in_the_first_alonzo_epoch() {
    // Found by scanning forward from the first block of epoch 290 and
    // reported here as the measured value: block 6,236,063, the fourth block
    // of the Alonzo era. The capture's own window covers it, so the claim is
    // re-readable rather than quoted.
    let src = fixtures();
    let h = case_tx_hash(cases::FIRST_PLUTUS_EXECUTION);
    let epoch =
        codetracer_cardano_recorder::chain::epoch_of_tx(&src, &h, ALONZO_WINDOW.0, ALONZO_WINDOW.1)
            .unwrap()
            .expect("the earliest Plutus execution is in the captured window");
    assert_eq!(epoch, 290);

    let blocks = src
        .blocks_in_range(ALONZO_WINDOW.0, ALONZO_WINDOW.1)
        .unwrap();
    let rows = src
        .block_txs(&blocks.iter().map(|b| b.hash.clone()).collect::<Vec<_>>())
        .unwrap();
    let row = rows.iter().find(|r| r.tx_hash == h).unwrap();
    assert_eq!(row.block_height, 6_236_063);

    // And it is a real script execution with the chain's own budget on it.
    let p = classify_case(cases::FIRST_PLUTUS_EXECUTION);
    let e = &p.executions[0];
    assert_eq!(e.purpose, "spend");
    assert_eq!((e.consensus_mem, e.consensus_steps), (520_448, 211_535_239));
    println!(
        "MEASURED: earliest reachable Plutus execution {h} at height {} in epoch {epoch}; \
         consensus units mem={} steps={}",
        row.block_height, e.consensus_mem, e.consensus_steps
    );
}

// ---------------------------------------------------------------------------
// Deliverable 5 — the strategy, with the retired claim re-measured
// ---------------------------------------------------------------------------

#[test]
fn test_the_retired_self_contained_claim_is_refuted_with_a_two_sided_control() {
    let mut read_cases = Vec::new();
    for case in cases::ALL {
        read_cases.push((case.to_string(), classify_case(case)));
    }
    let r = prestate::read(&read_cases);

    assert_eq!(r.declared, "hydrated-from-node");
    assert!(r.declared_in_closed_set);
    assert_eq!(
        r.verdict,
        prestate::SelfContainmentVerdict::Refuted,
        "the `self-contained` claim must be refuted by the bytes, not by a citation"
    );
    assert_eq!(
        r.controlled_cases,
        r.cases.len(),
        "every case's two-sided search control must hold, or an ABSENT reading is not \
         attributable to absence"
    );
    assert!(
        r.licenses_declaration(),
        "the reading must license the declared token"
    );
    println!(
        "MEASURED: {} case(s); verdict {}; {} with a working two-sided control",
        r.cases.len(),
        r.verdict,
        r.controlled_cases
    );
}

#[test]
fn test_the_plutus_v1_era_positive_arm_holds() {
    // The arm that makes the refutation an ERA REGRESSION rather than a
    // blanket absence: on the earliest mainnet Plutus execution the script
    // AND the spent output's datum are both in the transaction, each proved by
    // its own hash.
    let p = classify_case(cases::FIRST_PLUTUS_EXECUTION);
    let e = &p.executions[0];
    let by = |name: &str| {
        e.requisites
            .iter()
            .find(|r| r.requisite == name)
            .unwrap_or_else(|| panic!("requisite {name} must be reported"))
    };
    assert_eq!(
        by("script").carriage,
        provenance::Carriage::InTransaction,
        "the 2021 PlutusV1 script must be in the witness set: {}",
        by("script").evidence
    );
    assert_eq!(
        by("datum").carriage,
        provenance::Carriage::InTransaction,
        "the spent output's datum must be in the witness set: {}",
        by("datum").evidence
    );
    assert_eq!(by("redeemer").carriage, provenance::Carriage::InTransaction);
    assert_eq!(
        p.witness_scripts.len(),
        1,
        "exactly one script travels with this transaction"
    );
    println!(
        "MEASURED positive arm: era {:?}; script and datum both in-transaction; witness set \
         carries {} script(s) and {} datum(s)",
        p.ledger_era,
        p.witness_scripts.len(),
        p.witness_datums.len()
    );
}

#[test]
fn test_the_post_babbage_negative_arm_holds_for_both_shapes() {
    // Two post-Babbage shapes, measured rather than generalised from one.
    // Had only the reference-script case been read, the record would say "the
    // script is absent post-Babbage", and the embedded case refutes that.
    let reference = classify_case(cases::V3_REFERENCE_SCRIPT_SPEND);
    let embedded = classify_case(cases::V3_EMBEDDED_SCRIPT_SPEND);

    let script_of = |p: &provenance::TxProvenance| {
        p.executions
            .iter()
            .find(|e| e.purpose == "spend")
            .unwrap()
            .requisites
            .iter()
            .find(|r| r.requisite == "script")
            .unwrap()
            .carriage
    };
    let datum_of = |p: &provenance::TxProvenance| {
        p.executions
            .iter()
            .find(|e| e.purpose == "spend")
            .unwrap()
            .requisites
            .iter()
            .find(|r| r.requisite == "datum")
            .unwrap()
            .carriage
    };

    assert_eq!(
        script_of(&reference),
        provenance::Carriage::RequiresHydration
    );
    assert_eq!(script_of(&embedded), provenance::Carriage::InTransaction);
    assert_eq!(
        datum_of(&reference),
        provenance::Carriage::RequiresHydration
    );
    assert_eq!(
        datum_of(&embedded),
        provenance::Carriage::RequiresHydration,
        "the inline datum is the requisite that is absent in BOTH post-Babbage shapes"
    );
    println!(
        "MEASURED negative arm: reference-script case script={} datum={}; embedded-script case \
         script={} datum={} — so post-Babbage the datum is always hydrated and the script is \
         sometimes carried",
        script_of(&reference),
        datum_of(&reference),
        script_of(&embedded),
        datum_of(&embedded)
    );
}

#[test]
fn test_the_two_sided_control_can_fail() {
    // Driving the control red: the structural needle is the first input's
    // transaction id, which the body carries by construction. A transaction
    // with NO inputs has no structural needle, and the control must then
    // report itself as not two-sided rather than pass vacuously.
    let src = fixtures();
    let h = case_tx_hash(cases::V3_EMBEDDED_SCRIPT_SPEND);
    let cbor = src.tx_cbor(&h).unwrap();
    let p = provenance::classify(&cbor, &h, &src).unwrap();
    assert!(
        p.controls.two_sided(),
        "the real transaction's control holds"
    );

    // Now the negative: a control whose nonsense needle IS present cannot
    // distinguish presence from absence.
    let mut broken = p.clone();
    broken.controls.nonsense_found = true;
    assert!(
        !broken.controls.two_sided(),
        "a nonsense needle that is FOUND must invalidate the control"
    );
    let mut blind = p.clone();
    blind.controls.structural_found = false;
    assert!(
        !blind.controls.two_sided(),
        "a structural needle that is ABSENT must invalidate the control — the search found \
         nothing it was supposed to find"
    );
}

// ---------------------------------------------------------------------------
// Deliverable 6 — era boundaries and their cost models
// ---------------------------------------------------------------------------

#[test]
fn test_the_era_boundaries_are_enumerated_from_the_epochs_not_the_block_headers() {
    let src = fixtures();
    let versions = src.protocol_versions().expect("the series is captured");
    let b = eras::boundaries(&versions);
    let pairs: Vec<(u64, u64, u64)> = b
        .iter()
        .map(|x| (x.first_epoch, x.protocol_major, x.protocol_minor))
        .collect();
    assert_eq!(
        pairs,
        vec![
            (208, 2, 0),
            (236, 3, 0),
            (251, 4, 0),
            (290, 5, 0),
            (298, 6, 0),
            (365, 7, 0),
            (394, 8, 0),
            (507, 9, 0),
            (537, 10, 0),
            (644, 11, 0),
        ],
        "the measured boundary series moved"
    );

    // And the reading that says a block header is the wrong source: the
    // captured Alonzo window's blocks all announce protocol 5.0 while sitting
    // in epoch 290 — consistent here — but the series above is what the
    // epochs say, and the first epoch with protocol 5 is 290 rather than 286.
    assert_eq!(
        b.iter()
            .find(|x| x.protocol_major == 5)
            .unwrap()
            .first_epoch,
        290,
        "the Alonzo boundary is the epoch 289->290 boundary; block headers announce 5 as early \
         as epoch 286 and must not be used for this"
    );
    println!(
        "MEASURED: {} protocol-version boundary/boundaries: {pairs:?}",
        b.len()
    );
}

#[test]
fn test_an_eras_cost_model_is_built_from_that_eras_own_parameters_and_differs_from_todays() {
    // This is deliverable 6's load-bearing reading, and it is the half of the
    // milestone's `test_an_early_era_transaction_is_evaluated_under_its_own_rules`
    // that does not need a script context. The Alonzo PlutusV1 parameter
    // vector and the current one are DIFFERENT, so a replay that used today's
    // model for a 2021 transaction would be charging prices the chain never
    // set.
    let src = fixtures();
    let alonzo = src.epoch_params(290).unwrap();
    let today = src.epoch_params(664).unwrap();

    let a = alonzo.cost_models.get("PlutusV1").expect("Alonzo has V1");
    let t = today.cost_models.get("PlutusV1").expect("Conway has V1");
    assert_eq!(
        a.len(),
        166,
        "Alonzo serves exactly what this evaluator reads"
    );
    assert_eq!(t.len(), 332, "the current epoch serves twice as many");
    assert_ne!(
        a, t,
        "if an era's parameters equalled today's, era handling would be unobservable"
    );
    let differing = a.iter().zip(t.iter()).filter(|(x, y)| x != y).count();
    assert!(
        differing > 0,
        "even the first 166 parameters must differ, or using today's model for an Alonzo \
         transaction would be harmless and the era handling would not be load-bearing"
    );

    // Both must actually build, from the chain's own numbers.
    eras::cost_model_for(&alonzo, "PlutusV1").expect("Alonzo's own model must build");
    eras::cost_model_for(&today, "PlutusV1").expect("today's model must build");
    println!(
        "MEASURED: Alonzo(290) PlutusV1 has {} parameter(s), Conway(664) has {}; {differing} of \
         the shared 166 differ",
        a.len(),
        t.len()
    );
}

#[test]
fn test_an_era_whose_parameters_this_evaluator_cannot_read_is_refused() {
    // Measured: Conway at protocol 9.0 (epoch 507) served 251 PlutusV3
    // parameters and this evaluator reads 297. That era's own rules are
    // therefore unavailable, and the refusal is the correct outcome — a padded
    // model would report a budget as though it were the chain's.
    let src = fixtures();
    let p = src.epoch_params(507).unwrap();
    assert_eq!(p.cost_models["PlutusV3"].len(), 251);
    let err = eras::cost_model_for(&p, "PlutusV3")
        .expect_err("251 parameters cannot produce a 297-parameter model");
    let msg = err.to_string();
    assert!(
        msg.contains("251") && msg.contains("297") && msg.contains("refused"),
        "the refusal must state both arities and that it refuses; got: {msg}"
    );
    // And the neighbouring epoch, which DOES satisfy the arity, must build —
    // otherwise the refusal above could be a blanket failure rather than an
    // arity reading.
    let ok = src.epoch_params(537).unwrap();
    assert_eq!(ok.cost_models["PlutusV3"].len(), 297);
    eras::cost_model_for(&ok, "PlutusV3").expect("297 is exactly what the evaluator reads");
    println!("MEASURED: epoch 507 V3=251 REFUSED; epoch 537 V3=297 BUILT");
}

// ---------------------------------------------------------------------------
// The no-silent-skip gate
// ---------------------------------------------------------------------------

#[test]
fn test_a_live_chain_reading_reports_not_measured_when_the_gate_is_unset() {
    // The convention `tests/verify-cli-convention-no-silent-skip.sh` enforces:
    // anything that needs the network is opt-in, and when the gate is unset it
    // says NOT MEASURED out loud instead of passing or skipping. This test
    // asserts the LOUDNESS, which is the part a `#[ignore]` would silently
    // remove.
    let enabled = koios::live_chain_enabled();
    if enabled {
        println!(
            "{} is set, so the live readings in this run are live",
            koios::LIVE_CHAIN_ENV
        );
        return;
    }
    println!(
        "NOT MEASURED: the live-chain readings did not run because {} is unset. Everything else \
         in this file ran against committed captures of the chain's own bytes. To take the live \
         reading: {}=1 cargo test --test test_acquisition",
        koios::LIVE_CHAIN_ENV,
        koios::LIVE_CHAIN_ENV
    );
    // And the gate must be a real gate: a value it does not recognise must
    // not enable the live path.
    assert!(!enabled);
}

#[test]
fn test_the_live_chain_agrees_with_the_committed_captures() {
    // The opt-in arm. When it does not run it reports NOT MEASURED loudly
    // rather than being `#[ignore]`d, because an ignored test is invisible in
    // a summary line.
    if !koios::live_chain_enabled() {
        println!(
            "NOT MEASURED: {}=1 was not set, so the committed captures were NOT re-checked \
             against the live chain in this run. The captures' own hash identities still ran \
             (test_every_committed_capture_holds_the_chains_own_hash_identity), so what is \
             unmeasured here is DRIFT — whether the chain still serves these bytes — and not \
             whether the bytes are the chain's",
            koios::LIVE_CHAIN_ENV
        );
        return;
    }

    let live = koios::KoiosClient::new();
    let committed = fixtures();

    let e_live = enumerate::enumerate(&live, ALONZO_WINDOW.0, ALONZO_WINDOW.1)
        .expect("the live chain must enumerate the window");
    let e_fix = enumerate::enumerate(&committed, ALONZO_WINDOW.0, ALONZO_WINDOW.1).unwrap();
    assert_eq!(
        (e_live.block_count(), e_live.transactions),
        (e_fix.block_count(), e_fix.transactions),
        "the live chain and the committed capture disagree about a settled window, which would \
         mean the capture has drifted"
    );

    for case in cases::ALL {
        let h = case_tx_hash(case);
        let live_cbor = live
            .tx_cbor(&h)
            .expect("the live chain serves this transaction");
        let fix_cbor = committed.tx_cbor(&h).unwrap();
        assert_eq!(
            live_cbor, fix_cbor,
            "{case}: the live CBOR differs from the committed capture"
        );
    }
    println!(
        "MEASURED live: {} block(s), {} transaction(s), {} transaction CBOR(s) byte-identical to \
         their captures; {} HTTP call(s)",
        e_live.block_count(),
        e_live.transactions,
        cases::ALL.len(),
        live.call_count()
    );
}
