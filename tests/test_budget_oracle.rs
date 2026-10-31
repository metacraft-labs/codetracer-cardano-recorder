//! The per-transaction execution-budget oracle (CXP-ADA-1 deliverable 4).
//!
//! # Mocking
//!
//! **Nothing here is mocked.** Per the workspace policy that every use of a
//! mock must be justified, the justification is that none was needed and none
//! would have been admissible: the whole point of this file is that the
//! expected figures come from outside this repository.
//!
//! What stands in for the network is [`FixtureSource`], and it is not a test
//! double. It returns Cardano mainnet's own response bytes, committed as text
//! under `tests/fixtures/koios/`, and every one of them declares a hash
//! identity that `codetracer-cardano-recorder fixtures` **recomputes** —
//! `blake2b-256` of a transaction body's CBOR against its id, `blake2b-256` of
//! a datum against its hash, `blake2b-224(tag ‖ script)` against a script
//! hash. A mock returns what its author decided; these return what the chain
//! returned, and the difference is checkable inside the file.
//!
//! The one place a doctored input appears is
//! `test_an_epoch_whose_cost_model_is_short_is_not_measured`, which builds an
//! [`EpochParams`] by hand to stand in for epoch 507 and is labelled as such.
//! It is not a mock of the oracle; it is the input that must be **refused**.
//!
//! # The oracle
//!
//! A Cardano transaction's witness set declares, per redeemer, the execution
//! units that redeemer may spend. The ledger's phase-two rule is that
//! execution must fit inside the declaration, and a settled transaction's did.
//! The submitter chose the figure and the network validated it, so **no bug in
//! this recorder can move it**. That is the class of oracle
//! `Debug-Fidelity-Adversarial-Testing.md` specifies, and it is the strongest
//! one available on any chain in this family.
//!
//! # What the relation actually is, measured rather than hoped for
//!
//! Equality is **not** the relation in general, and this was measured before
//! the assertion was written. Over 463 live mainnet transactions (the ten
//! captured Alonzo-era blocks, 330 transactions, plus heights
//! 14,139,040–14,139,072, 133 transactions), 39 carried a script execution and
//! 64 redeemers replayed: **58 EQUAL, 6 UNDER, 0 OVER, 0 ERROR, 0 NOT
//! MEASURED**. All six under-declared redeemers are PlutusV2, and they are two
//! different shapes rather than one: three carry a round over-estimate
//! (`2,000,000 / 1,200,000,000`, `100,000 / 30,000,000`,
//! `420,000 / 160,000,000`), and three are simulated-looking declarations by a
//! single script that each over-shoot by the *same* `mem 10,982 /
//! steps 2,876,466` — a constant margin on the submitter's side, which the 58
//! EQUAL readings are what rule out being a systematic under-charge here. The
//! committed corpus deliberately includes one under-declared transaction —
//! `82dc8232…`, all three of whose redeemers are UNDER — so that this file
//! cannot drift into asserting equality.
//!
//! So the standing assertion is in two parts, and they are different in kind:
//!
//! 1. **the oracle**: per redeemer, `replayed <= declared` on both axes, and
//!    `replayed == declared` for the redeemers whose declaration was exact.
//!    The right-hand side is the chain's. This is what generalises to any
//!    transaction, and it is what the live census arm asserts.
//! 2. **a regression pin**: for the three under-declared redeemers the chain's
//!    figure only *bounds* the budget, so the exact slack is pinned from the
//!    measurement. **That half is a regression guard and not an oracle**, it is
//!    labelled as such in [`CORPUS`], and it exists because of what the
//!    control measured: see below.
//!
//! # The control, and what it revealed about the oracle's strength
//!
//! The milestone's control clause is "an injected off-by-one in the budget
//! accounting is caught by the same comparison". Driven red here by
//! [`AccountingFault::off_by_one_cpu`], which perturbs the budget the recorder
//! reads off the machine — every per-step reading and so the total — rather
//! than the comparison that consumes it. Perturbing the comparison would
//! prove only that `!=` works.
//!
//! Measured: the ledger-rule assertion alone catches the off-by-one on **6 of
//! the 9** corpus redeemers and misses 3. That is not a weak control, it is a
//! true property of the oracle, and stating it is the point: on an
//! over-declared redeemer the chain's figure bounds the budget and does not
//! determine it, so one extra CPU unit still fits. The exact-relation pin
//! closes the remaining 3, giving **9 of 9**. Both numbers are asserted, so
//! neither can quietly change.
//!
//! # What this file does NOT observe
//!
//! - **No network and no credential**, unless
//!   `CODETRACER_CARDANO_RECORDER_LIVE_CHAIN=1` is set, and then only in the
//!   one test that says so. Without it that test prints **NOT MEASURED** and
//!   names what is unmeasured rather than skipping; there is no `#[ignore]` in
//!   this file and `tests/verify-cli-convention-no-silent-skip.sh` refuses one.
//! - **No trace, no `.ct` bundle.** This file measures budgets and step
//!   counts. Nothing here writes a recording.
//! - **Not `src/tracer.rs`.** The source-simulating interpreter is not
//!   reachable from any path in this file, and removing it is deliverable 2.
//! - **Not the Aiken compiler.** The scripts executed are the UPLC the chain
//!   stores. Compiling Aiken source is deliverable 3 and nothing here stands
//!   in for it.

use std::collections::BTreeMap;

use codetracer_cardano_recorder::cek::{self, AccountingFault};
use codetracer_cardano_recorder::chain::{ChainSource, EpochParams};
use codetracer_cardano_recorder::eras::cost_model_for;
use codetracer_cardano_recorder::fixture::{cases, FixtureSource};
use codetracer_cardano_recorder::oracle::{self, BudgetRelation};
use pallas_codec::minicbor;
use pallas_primitives::conway::{Language, MintedTx};
use pallas_primitives::Fragment;
use uplc::ast::{FakeNamedDeBruijn, NamedDeBruijn, Program};
use uplc::machine::cost_model::ExBudget;
use uplc::tx::iter_redeemers;
use uplc::tx::script_context::{
    find_script, DataLookupTable, PlutusScript, ScriptContext, SlotConfig, TxInfoV3,
};
use uplc::tx::to_plutus_data::ToPlutusData;

// ---------------------------------------------------------------------------
// The corpus, as measured
// ---------------------------------------------------------------------------

/// One redeemer's expected reading.
struct Expect {
    tx: &'static str,
    purpose: &'static str,
    index: u32,
    language: &'static str,
    /// What was applied to the script, which differs by Plutus version.
    application: &'static str,
    /// `None` — the replay equals the chain's declaration exactly. **This is
    /// the oracle**: the right-hand side is the network's figure.
    ///
    /// `Some((mem, steps))` — the declaration is an over-estimate and the
    /// chain's figure only bounds the replay. The slack is pinned from the
    /// measurement, so **this is a regression guard, not an oracle**. It is
    /// here because without it the control's off-by-one would go uncaught on
    /// these three redeemers.
    under: Option<(u64, u64)>,
}

const FIRST_PLUTUS: &str = "a95d16e891e51f98a3b1d3fe862ed355ebc8abffb7a7269d86f775553d9e653f";
const V1_MINT: &str = "e9d20746cec2d506a5ae201adbfec13ef20279b689f0612cc3802274ce4da2ce";
const V3_REF: &str = "b116d0a734c0369d79062004618dffbf5b6770e76a28114640913aa71eacbbac";
const V3_EMBEDDED: &str = "0ab94a1542a36f2be087627f5e1c7cdfeb26bc1b8672004a760a1fed93f3c336";
const V2_OVER: &str = "82dc8232feaf701b97e3fe082d5909fd3c437f4adc6c37c7634bd8782a49b0ca";

/// Every redeemer the committed corpus carries, with the reading measured on
/// 2026-10-31.
///
/// The corpus is chosen for coverage rather than for size: all three Plutus
/// languages, all three application orders, three purposes (`spend`, `mint`,
/// `reward`), both eras whose cost models differ, scripts carried in the
/// witness set and scripts on CIP-33 reference outputs, datums carried by hash
/// and datums inline, and both sides of the relation.
const CORPUS: &[Expect] = &[
    // Alonzo, PlutusV1, the earliest Plutus execution reachable on mainnet.
    // Its spending order is the three-argument one.
    Expect {
        tx: FIRST_PLUTUS,
        purpose: "spend",
        index: 0,
        language: "PlutusV1",
        application: "((script datum) redeemer) context",
        under: None,
    },
    // Alonzo, PlutusV1, a MINTING policy: two arguments, not three.
    Expect {
        tx: V1_MINT,
        purpose: "mint",
        index: 0,
        language: "PlutusV1",
        application: "(script redeemer) context",
        under: None,
    },
    // Conway, PlutusV3: the context carries the redeemer and the datum, so
    // only the context is applied.
    Expect {
        tx: V3_REF,
        purpose: "spend",
        index: 0,
        language: "PlutusV3",
        application: "script context",
        under: None,
    },
    Expect {
        tx: V3_REF,
        purpose: "spend",
        index: 2,
        language: "PlutusV3",
        application: "script context",
        under: None,
    },
    Expect {
        tx: V3_REF,
        purpose: "reward",
        index: 0,
        language: "PlutusV3",
        application: "script context",
        under: None,
    },
    Expect {
        tx: V3_EMBEDDED,
        purpose: "spend",
        index: 0,
        language: "PlutusV3",
        application: "script context",
        under: None,
    },
    // Conway, PlutusV2, over-declared on every redeemer. The slacks below are
    // pinned measurements and not the chain's statement — see `Expect::under`.
    Expect {
        tx: V2_OVER,
        purpose: "spend",
        index: 0,
        language: "PlutusV2",
        application: "((script datum) redeemer) context",
        under: Some((1_451_396, 1_010_332_195)),
    },
    Expect {
        tx: V2_OVER,
        purpose: "spend",
        index: 1,
        language: "PlutusV2",
        application: "((script datum) redeemer) context",
        under: Some((17_656, 3_861_446)),
    },
    Expect {
        tx: V2_OVER,
        purpose: "reward",
        index: 0,
        language: "PlutusV2",
        application: "(script redeemer) context",
        under: Some((224_210, 94_106_508)),
    },
];

/// How many corpus redeemers the chain's figure pins exactly, and how many it
/// only bounds. Asserted below so the split cannot drift unnoticed.
const EXACT_REDEEMERS: usize = 6;
const UNDER_REDEEMERS: usize = 3;

fn fixtures() -> FixtureSource {
    FixtureSource::at_crate_root()
}

fn corpus_hashes() -> Vec<String> {
    let fx = fixtures();
    cases::ALL_WITH_BUDGET
        .iter()
        .map(|case| {
            fx.load(&format!("tx_cbor_{case}"))
                .unwrap_or_else(|e| panic!("capture tx_cbor_{case}: {e}"))
                .identity["expect_tx_hash"]
                .as_str()
                .expect("every tx_cbor capture declares expect_tx_hash")
                .to_string()
        })
        .collect()
}

/// Every reading, keyed so an expectation can be looked up rather than
/// matched positionally — a positional match would pass if two redeemers
/// swapped.
fn readings_by_key(
    census: &oracle::Census,
) -> BTreeMap<(String, String, u32), oracle::RedeemerBudget> {
    let mut out = BTreeMap::new();
    for r in &census.readings {
        for b in &r.redeemers {
            out.insert(
                (r.tx_hash.clone(), b.purpose.to_string(), b.index),
                b.clone(),
            );
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The standing check
// ---------------------------------------------------------------------------

#[test]
fn test_the_replayed_budget_matches_the_chains() {
    let src = fixtures();
    let hashes = corpus_hashes();
    let census = oracle::census_over_hashes(&src, &hashes, AccountingFault::none());

    // A check over an empty or shrunken corpus is the thing this whole
    // convention exists to rule out, so the population is pinned first.
    assert_eq!(
        census.transactions_read,
        CORPUS
            .iter()
            .map(|e| e.tx)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        "the corpus size changed; a budget check over a shrunken corpus would pass for the \
         wrong reason"
    );
    assert!(
        census.not_measured.is_empty(),
        "every corpus transaction must be measurable offline; NOT MEASURED: {:#?}",
        census.not_measured
    );
    assert_eq!(
        census.redeemer_count(),
        CORPUS.len(),
        "expected {} redeemer(s) across the corpus, replayed {}",
        CORPUS.len(),
        census.redeemer_count()
    );

    let by_key = readings_by_key(&census);
    for e in CORPUS {
        let b = by_key
            .get(&(e.tx.to_string(), e.purpose.to_string(), e.index))
            .unwrap_or_else(|| {
                panic!(
                    "no reading for {} {}[{}]; an absent redeemer reads as satisfied",
                    e.tx, e.purpose, e.index
                )
            });

        assert_eq!(
            b.language, e.language,
            "{} {}[{}]: the script's language decides which cost model and which application \
             order are used, so a wrong one is not a detail",
            e.tx, e.purpose, e.index
        );
        assert_eq!(
            b.application.to_string(),
            e.application,
            "{} {}[{}]: the arguments applied to the script differ by Plutus version, and a \
             swapped order produces a program that evaluates and is wrong",
            e.tx,
            e.purpose,
            e.index
        );
        assert!(
            !b.errored,
            "{} {}[{}]: the script evaluated to ERROR. A settled transaction's phase-two \
             scripts succeeded, so this is a divergence whatever the budget says",
            e.tx, e.purpose, e.index
        );

        // THE ORACLE: the ledger's own rule, against the network's figure.
        assert!(
            b.relation.fits(),
            "{} {}[{}]: replayed MORE than the transaction declares ({}). By the ledger's own \
             phase-two rule this transaction could not have settled, so either the replay is \
             wrong or the chain is",
            e.tx,
            e.purpose,
            e.index,
            b.relation
        );

        match e.under {
            None => assert_eq!(
                b.relation,
                BudgetRelation::Equal,
                "{} {}[{}]: declared mem={} steps={} and replayed mem={} steps={} — this \
                 redeemer's declaration is the exact figure the chain charged, so equality is \
                 the oracle and not a tolerance",
                e.tx,
                e.purpose,
                e.index,
                b.declared.mem,
                b.declared.steps,
                b.replayed.mem,
                b.replayed.cpu
            ),
            Some((mem, steps)) => assert_eq!(
                b.relation,
                BudgetRelation::Under { mem, steps },
                "{} {}[{}]: this redeemer is over-declared, so the chain's figure only BOUNDS \
                 the budget and the slack below is a pinned measurement rather than the chain's \
                 statement. It changed",
                e.tx,
                e.purpose,
                e.index
            ),
        }
    }

    assert_eq!(
        (
            census.exact_count(),
            census.under_count(),
            census.over_count()
        ),
        (EXACT_REDEEMERS, UNDER_REDEEMERS, 0),
        "the split between redeemers the chain pins exactly and redeemers it only bounds is \
         itself a measurement; if it moved, the record's wording about what `agreement` means \
         here is stale"
    );

    println!(
        "MEASURED: {} committed transaction(s), {} redeemer(s) replayed by stepping the chain's \
         CEK machine under each transaction's own epoch's cost model: {} EQUAL to the units the \
         chain records, {} UNDER (over-declared by the submitter), {} OVER, {} NOT MEASURED",
        census.transactions_read,
        census.redeemer_count(),
        census.exact_count(),
        census.under_count(),
        census.over_count(),
        census.not_measured.len()
    );
}

// ---------------------------------------------------------------------------
// The control, driven red
// ---------------------------------------------------------------------------

#[test]
fn test_an_injected_off_by_one_in_the_budget_accounting_is_caught() {
    let src = fixtures();
    let hashes = corpus_hashes();

    // The fault perturbs what the recorder reads off the machine, not the
    // comparison. One extra CPU unit, claimed as spent.
    let faulted = oracle::census_over_hashes(&src, &hashes, AccountingFault::off_by_one_cpu());
    assert_eq!(
        faulted.redeemer_count(),
        CORPUS.len(),
        "the faulted run must still replay every redeemer, or the control would be measuring a \
         failure to run instead of a failure to agree"
    );

    // Arm 1 — the ORACLE alone. An exactly-declared redeemer becomes OVER,
    // which the ledger's rule catches against a figure the network produced.
    let caught_by_the_oracle = faulted
        .readings
        .iter()
        .flat_map(|r| &r.redeemers)
        .filter(|b| !b.relation.fits())
        .count();
    assert_eq!(
        caught_by_the_oracle, EXACT_REDEEMERS,
        "the ledger-rule assertion must catch the off-by-one on exactly the {EXACT_REDEEMERS} \
         redeemers the chain pins exactly"
    );

    // Arm 2 — and it misses the over-declared ones, which is a true property
    // of the oracle rather than a weakness of the control: one extra unit
    // still fits inside a round over-estimate.
    let missed_by_the_oracle = faulted
        .readings
        .iter()
        .flat_map(|r| &r.redeemers)
        .filter(|b| b.relation.fits())
        .count();
    assert_eq!(
        missed_by_the_oracle, UNDER_REDEEMERS,
        "on an over-declared redeemer the chain's figure BOUNDS the budget and does not \
         determine it, so the ledger rule cannot see a one-unit error there. If this number \
         moved, the oracle's strength has changed and the record must say so"
    );

    // Arm 3 — every redeemer, through the standing check's second half. The
    // relation each redeemer was measured to have must have changed.
    let by_key = readings_by_key(&faulted);
    let mut changed = 0usize;
    for e in CORPUS {
        let b = &by_key[&(e.tx.to_string(), e.purpose.to_string(), e.index)];
        let expected = match e.under {
            None => BudgetRelation::Equal,
            Some((mem, steps)) => BudgetRelation::Under { mem, steps },
        };
        assert_ne!(
            b.relation, expected,
            "{} {}[{}]: a +1 CPU error in the accounting left the relation unchanged, so the \
             standing check would not have noticed it. That makes agreement a tautology",
            e.tx, e.purpose, e.index
        );
        changed += 1;
        // And the error is exactly one unit on the CPU axis, in the direction
        // injected: a control that fired for some other reason would not be
        // this control.
        assert_eq!(
            b.replayed.cpu - 1,
            clean_cpu(&src, e),
            "{} {}[{}]: the faulted replay must differ from the clean one by exactly the \
             injected unit",
            e.tx,
            e.purpose,
            e.index
        );
    }
    assert_eq!(changed, CORPUS.len());

    println!(
        "CONTROL DRIVEN RED: a +1 CPU error injected into the recorder's budget accounting \
         changes the measured relation on {}/{} corpus redeemers. The ledger-rule arm alone \
         catches {} of them (the exactly-declared ones) and misses {} (the over-declared ones, \
         where the chain's figure bounds rather than determines the budget)",
        changed,
        CORPUS.len(),
        caught_by_the_oracle,
        missed_by_the_oracle
    );
}

/// The clean replay's CPU figure for one expectation, so the control can show
/// the injected unit is the whole of the difference.
fn clean_cpu(src: &FixtureSource, e: &Expect) -> i64 {
    let epoch = src.tx_epoch(e.tx).expect("the corpus epochs are captured");
    let r = oracle::replay_from_source(src, e.tx, epoch, AccountingFault::none())
        .expect("the clean replay succeeds");
    r.redeemers
        .iter()
        .find(|b| b.purpose == e.purpose && b.index == e.index)
        .expect("the redeemer is present in the clean run")
        .replayed
        .cpu
}

// ---------------------------------------------------------------------------
// The era's own cost model is load-bearing
// ---------------------------------------------------------------------------

#[test]
fn test_an_early_era_transaction_is_evaluated_under_its_own_rules() {
    // This closes CXP-ADA-2's entry of the same name on the clause it was
    // failing: "and its budget matches the chain's". The Alonzo transaction's
    // budget is replayed under epoch 290's own PlutusV1 parameters and equals
    // the units the chain recorded.
    let src = fixtures();
    let epoch = src.tx_epoch(FIRST_PLUTUS).unwrap();
    assert_eq!(epoch, 290, "this case is the earliest Plutus execution");
    let r = oracle::replay_from_source(&src, FIRST_PLUTUS, epoch, AccountingFault::none()).unwrap();
    assert_eq!(r.ledger_era.as_deref(), Some("Alonzo"));
    assert_eq!(r.redeemers.len(), 1);
    assert_eq!(r.redeemers[0].relation, BudgetRelation::Equal);

    // The control the entry specifies: charge the SAME transaction under the
    // current era's rules and the budget stops matching. Without this, the
    // agreement above could be an accident of the two models being equal.
    let now = src.epoch_params(664).unwrap();
    let alonzo = src.epoch_params(290).unwrap();
    let cbor = src.tx_cbor(FIRST_PLUTUS).unwrap();
    let rows = src
        .utxos(&[codetracer_cardano_recorder::chain::UtxoRef::new(
            "7510385def92ad50602b75e519f733ed2acd248610aa5f920218ce8010acd4b4",
            0,
        )])
        .unwrap();

    assert_ne!(
        alonzo.cost_models["PlutusV1"][..166],
        now.cost_models["PlutusV1"][..166],
        "if the two eras' PlutusV1 parameters were equal there would be nothing for the era \
         handling to be load-bearing about, and this control would be decoration"
    );

    let wrong_era = oracle::replay_tx(
        &cbor,
        FIRST_PLUTUS,
        &rows,
        &now,
        &SlotConfig::default(),
        AccountingFault::none(),
    )
    .expect("the replay still runs; it is the budget that is wrong");
    assert_ne!(
        wrong_era.redeemers[0].relation,
        BudgetRelation::Equal,
        "charging a 2021 Alonzo transaction under epoch 664's prices must NOT reproduce the \
         units the chain recorded, or the cost model is not being used"
    );
    println!(
        "MEASURED: epoch 290's own PlutusV1 model reproduces the chain's units exactly \
         (mem={} steps={}); epoch 664's model gives {} — so the era handling is load-bearing",
        r.redeemers[0].replayed.mem, r.redeemers[0].replayed.cpu, wrong_era.redeemers[0].relation
    );
}

#[test]
fn test_an_epoch_whose_cost_model_is_short_is_not_measured() {
    // Epoch 507 serves 251 PlutusV3 parameters and this evaluator reads 297.
    // `cost_model_for` refuses rather than padding, and the refusal must reach
    // the caller as NOT MEASURED instead of being charged under some other
    // era's prices. The `EpochParams` below is built by hand — see this file's
    // mocking note — because the point is the input that must be refused.
    let src = fixtures();
    let short = EpochParams {
        epoch_no: 507,
        era: Some("Conway".into()),
        protocol_major: Some(9),
        protocol_minor: Some(0),
        cost_models: BTreeMap::from([("PlutusV3".to_string(), vec![1i64; 251])]),
    };
    assert!(cost_model_for(&short, "PlutusV3").is_err());

    let cbor = src.tx_cbor(V3_EMBEDDED).unwrap();
    let tx: MintedTx = minicbor::decode(&cbor).unwrap();
    let refs: Vec<_> = tx
        .transaction_body
        .inputs
        .iter()
        .map(|i| {
            codetracer_cardano_recorder::chain::UtxoRef::new(i.transaction_id.to_string(), i.index)
        })
        .collect();
    let rows = src.utxos(&refs).unwrap();

    let err = oracle::replay_tx(
        &cbor,
        V3_EMBEDDED,
        &rows,
        &short,
        &SlotConfig::default(),
        AccountingFault::none(),
    )
    .expect_err("a short parameter vector must refuse rather than approximate");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("251") && msg.contains("297") && msg.contains("refused"),
        "the refusal must name both arities and say it refused; got: {msg}"
    );
    println!("MEASURED: a 251-parameter PlutusV3 epoch is NOT MEASURED, not padded: {msg}");
}

// ---------------------------------------------------------------------------
// The script context is reconstructed, and the placeholder is gone
// ---------------------------------------------------------------------------

#[test]
fn test_the_script_context_is_assembled_from_the_transaction_and_its_inputs() {
    // The clause this closes: `src/blockfrost.rs` used to set
    // `script_context: String::new()`. A real context is assembled here, and
    // it is asserted to carry the transaction's own id — which is the field a
    // placeholder could not have.
    let src = fixtures();
    let cbor = src.tx_cbor(V3_EMBEDDED).unwrap();
    let tx: MintedTx = minicbor::decode(&cbor).unwrap();
    let refs: Vec<_> = tx
        .transaction_body
        .inputs
        .iter()
        .map(|i| {
            codetracer_cardano_recorder::chain::UtxoRef::new(i.transaction_id.to_string(), i.index)
        })
        .collect();
    let utxos = oracle::resolved_inputs(&src.utxos(&refs).unwrap()).unwrap();
    let lookup = DataLookupTable::from_transaction(&tx, &utxos);

    let redeemers = tx.transaction_witness_set.redeemer.as_ref().unwrap();
    let (key, data, ex_units) = iter_redeemers(redeemers).next().unwrap();
    let redeemer = pallas_primitives::conway::Redeemer {
        tag: key.tag,
        index: key.index,
        data: data.clone(),
        ex_units,
    };
    let (_script, datum) = find_script(&redeemer, &tx, &utxos, &lookup).unwrap();
    let ctx = TxInfoV3::from_transaction(&tx, &utxos, &SlotConfig::default())
        .unwrap()
        .into_script_context(&redeemer, datum.as_ref())
        .expect("a V3 script context must be constructible for a settled V3 transaction");

    let encoded = pallas_primitives::conway::PlutusData::encode_fragment(&ctx.to_plutus_data())
        .expect("the context is PlutusData and encodes");
    assert!(
        encoded.len() > 200,
        "the context encoded to {} byte(s); the placeholder it replaces was zero and a context \
         that small would not carry a transaction body",
        encoded.len()
    );
    assert!(
        hex::encode(&encoded).contains(V3_EMBEDDED),
        "the reconstructed context must carry the transaction's own id; an empty or synthetic \
         context could not"
    );
    println!(
        "MEASURED: the reconstructed PlutusV3 script context is {} byte(s) of PlutusData and \
         carries the transaction id {V3_EMBEDDED}",
        encoded.len()
    );
}

#[test]
fn test_applying_the_v1v2_order_to_a_v3_script_is_caught() {
    // The claim under test was "getting the application order wrong produces
    // a trace that still runs", and the measurement (2026-10-31) says
    // something more specific: on THIS script the swap does not run — the
    // machine fails at step 63 with `EmptyList`, because the extra argument
    // shifts what the script destructures. So the swap is caught by the
    // evaluator here and would be caught by the budget on a script that
    // tolerated the extra argument. This test asserts "caught, either way"
    // and PRINTS which arm caught it, rather than asserting a failure mode
    // that was not observed.
    let src = fixtures();
    let cbor = src.tx_cbor(V3_EMBEDDED).unwrap();
    let tx: MintedTx = minicbor::decode(&cbor).unwrap();
    let refs: Vec<_> = tx
        .transaction_body
        .inputs
        .iter()
        .map(|i| {
            codetracer_cardano_recorder::chain::UtxoRef::new(i.transaction_id.to_string(), i.index)
        })
        .collect();
    let utxos = oracle::resolved_inputs(&src.utxos(&refs).unwrap()).unwrap();
    let lookup = DataLookupTable::from_transaction(&tx, &utxos);

    let redeemers = tx.transaction_witness_set.redeemer.as_ref().unwrap();
    let (key, data, declared) = iter_redeemers(redeemers).next().unwrap();
    let redeemer = pallas_primitives::conway::Redeemer {
        tag: key.tag,
        index: key.index,
        data: data.clone(),
        ex_units: declared,
    };
    let (script, datum) = find_script(&redeemer, &tx, &utxos, &lookup).unwrap();
    let PlutusScript::V3(bytes) = &script else {
        panic!("this case is PlutusV3");
    };
    let ctx = TxInfoV3::from_transaction(&tx, &utxos, &SlotConfig::default())
        .unwrap()
        .into_script_context(&redeemer, datum.as_ref())
        .unwrap();
    assert!(
        matches!(ctx, ScriptContext::V3 { .. }),
        "a V3 transaction's context must be the V3 shape, or the arm below is not the wrong one"
    );

    let mut buf = Vec::new();
    let program: Program<NamedDeBruijn> =
        Program::<FakeNamedDeBruijn>::from_cbor(&bytes.0, &mut buf)
            .unwrap()
            .into();
    // The WRONG order: the V1V2 sequence, applied to a V3 script.
    let wrong = program
        .apply_data(redeemer.data.clone())
        .apply_data(ctx.to_plutus_data());

    let costs = cost_model_for(&src.epoch_params(664).unwrap(), "PlutusV3").unwrap();
    let outcome = cek::step_program(wrong, &Language::PlutusV3, costs, ExBudget::default());

    match outcome {
        Ok(run) => {
            let replayed = run.consumed();
            let relation = oracle::relate(replayed, declared);
            assert_ne!(
                relation,
                BudgetRelation::Equal,
                "the wrong application order reproduced the chain's budget exactly, which would \
                 mean the budget comparison cannot see the difference between applying a V3 \
                 context and applying a redeemer and a context"
            );
            println!(
                "MEASURED: the V1V2 order applied to a V3 script ran to completion in {} CEK \
                 transitions and produced {relation} — a trace that still runs and that the \
                 budget comparison catches",
                run.step_count()
            );
        }
        Err(e) => {
            // The Err arm is the one this case actually takes, so it must
            // assert and not only print: without this, any failure at all
            // would satisfy the test, including one that had nothing to do
            // with the swapped argument. What is asserted is that the CEK
            // MACHINE is what refused — the program was built, decoded and
            // stepped, and it broke under evaluation — rather than the
            // program failing to assemble for some unrelated reason.
            let msg = format!("{e:#}");
            assert!(
                msg.contains("CEK machine failed at step"),
                "the swap must be caught by the machine or by the budget; this failure is \
                 neither, so the test would be passing on an unrelated error: {msg}"
            );
            println!(
                "MEASURED: the V1V2 order applied to a V3 script failed in the machine rather \
                 than running: {msg}. Caught either way; the budget arm is the one that catches \
                 the cases that do run"
            );
        }
    }
}

#[test]
fn test_a_partially_hydrated_transaction_is_refused_rather_than_measured() {
    // A context assembled from fewer inputs than the transaction names is a
    // context the chain never hashed. Measuring against it would produce a
    // divergence with an innocent-looking cause, so it is refused.
    let src = fixtures();
    let cbor = src.tx_cbor(V3_REF).unwrap();
    let params = src.epoch_params(664).unwrap();
    let tx: MintedTx = minicbor::decode(&cbor).unwrap();
    let mut refs: Vec<_> = tx
        .transaction_body
        .inputs
        .iter()
        .map(|i| {
            codetracer_cardano_recorder::chain::UtxoRef::new(i.transaction_id.to_string(), i.index)
        })
        .collect();
    for i in tx
        .transaction_body
        .reference_inputs
        .as_ref()
        .unwrap()
        .iter()
    {
        refs.push(codetracer_cardano_recorder::chain::UtxoRef::new(
            i.transaction_id.to_string(),
            i.index,
        ));
    }
    let all = src.utxos(&refs).unwrap();
    assert!(all.len() > 1);

    // Withhold one leaf.
    let short = &all[1..];
    let err = oracle::replay_tx(
        &cbor,
        V3_REF,
        short,
        &params,
        &SlotConfig::default(),
        AccountingFault::none(),
    )
    .expect_err("a transaction missing a resolved input must be refused");
    println!(
        "MEASURED: withholding one of {} hydrated leaves refuses the replay: {err:#}",
        all.len()
    );
}

// ---------------------------------------------------------------------------
// The live arm: drift, not authenticity
// ---------------------------------------------------------------------------

#[test]
fn test_the_live_chain_agrees_with_the_committed_budget_readings() {
    use codetracer_cardano_recorder::koios;

    if !koios::live_chain_enabled() {
        // NOT MEASURED, loudly, and naming what is unmeasured. The committed
        // readings' authenticity is measured by the test above, offline;
        // what is unmeasured here is DRIFT — whether the chain still answers
        // the way the captures say it did.
        println!(
            "NOT MEASURED: chain DRIFT. {}=1 re-reads every corpus transaction from the live \
             chain and re-runs the budget comparison against it. Unset, this test measures \
             nothing and says so; what is NOT in doubt is the committed captures' authenticity, \
             which test_the_replayed_budget_matches_the_chains measures offline against the \
             chain's own hashes.",
            koios::LIVE_CHAIN_ENV
        );
        return;
    }

    let live = koios::KoiosClient::new();
    let hashes = corpus_hashes();
    let census = oracle::census_over_hashes(&live, &hashes, AccountingFault::none());
    assert!(
        census.not_measured.is_empty(),
        "live NOT MEASURED: {:#?}",
        census.not_measured
    );
    assert_eq!(census.redeemer_count(), CORPUS.len());

    let offline = oracle::census_over_hashes(&fixtures(), &hashes, AccountingFault::none());
    let live_by_key = readings_by_key(&census);
    let offline_by_key = readings_by_key(&offline);
    for (k, b) in &offline_by_key {
        let l = live_by_key
            .get(k)
            .unwrap_or_else(|| panic!("the live chain served no reading for {k:?}"));
        assert_eq!(
            (
                l.replayed.mem,
                l.replayed.cpu,
                l.declared.mem,
                l.declared.steps
            ),
            (
                b.replayed.mem,
                b.replayed.cpu,
                b.declared.mem,
                b.declared.steps
            ),
            "{k:?}: the live chain and the committed capture disagree, so a capture has drifted"
        );
    }
    println!(
        "MEASURED live: {} transaction(s), {} redeemer(s), every replayed and declared figure \
         identical to the committed captures; {} HTTP call(s), no API key",
        census.transactions_read,
        census.redeemer_count(),
        live.call_count()
    );
}
