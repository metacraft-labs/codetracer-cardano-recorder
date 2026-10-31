//! The real Aiken compiler is invoked, and what it produced is what runs
//! (CXP-ADA-1 deliverable 3).
//!
//! # Mocking
//!
//! **Nothing here is mocked, and the one thing that could be mistaken for a
//! mock is not one.** Per the workspace policy that every use of a mock must
//! be justified in the test file's header, the justification is that none was
//! needed:
//!
//! - the front-end is `aiken-lang`'s own lexer, parser and type checker, and
//!   the code generator is its own `CodeGenerator::generate_raw` — the
//!   identical call `aiken_lang::test_framework::Test::unit_test` makes for a
//!   unit test;
//! - the machine is the `uplc` crate's CEK machine, reached through
//!   `crate::cek`;
//! - the storage encoding and the script hash come from
//!   `uplc::ast::SerializableProgram::compiled_code_and_hash`, which is the
//!   function Aiken's blueprint serializer itself calls;
//! - the cost model comes from `crate::fixture::FixtureSource`, which is a
//!   committed capture of the chain's own `/epoch_params` response and not a
//!   test double — every capture's identity is recomputed by
//!   `codetracer-cardano-recorder fixtures`, and the arity of the vector used
//!   here is checked against what the evaluator reads rather than assumed.
//!
//! # What these tests measure, and what they do NOT
//!
//! They measure that **the UPLC handed to the CEK machine is byte-identical to
//! what the real Aiken compiler produced for that module**, in the encoding
//! and under the hash identity the chain uses to store and name a script.
//!
//! **The machine's INPUT, not its run — and the difference is load-bearing.**
//! `compile::execute_artifact_with_fault` re-encodes a clone of the program
//! taken *before* `cek::step_program` is called, and `step_program` moves the
//! program into the machine and never writes back, so the byte comparison is
//! fixed before the first transition. It rules out a substitution between
//! `decode` and `execute`; it cannot see anything the run itself does.
//! Measured on review, 2026-10-31: substituting the executed term for a
//! constant after the comparison value is derived left every assertion in
//! `test_the_compiler_is_invoked_and_its_output_is_what_runs` green, still
//! reporting "15 of 15 executed the compiler's own bytes". What rules out
//! executing something else is the shape of that function — the cloned value
//! is moved straight into the machine — which is read from the code rather
//! than asserted here. `uplc`'s `Machine` exposes no final program to compare
//! against, so an assertion that observed the run is not available today.
//!
//! They do **not** compare against a script the chain stores, and the
//! distinction is deliberate rather than a shortfall. CXP-ADA-1's verification
//! entry for this deliverable is worded *"byte-compared against the script the
//! chain stores"*, which has a subject on the `replay` / `budget-oracle` path
//! and has none here: a local `.ak` file was never submitted, so no
//! transaction names it and no ledger carries its script hash. Inventing a
//! chain-side expectation for it would be a fabricated oracle.
//!
//! **The chain half of that clause is satisfied elsewhere and is not
//! re-satisfied here.** `tests/test_budget_oracle.rs` executes the chain's own
//! script bytes, decoded from committed transaction CBOR: there the executed
//! artifact *is* the stored script, so the identity holds by construction, and
//! the budgets agree with the units the chain recorded. What this file closes
//! is the other gap — the one §2.2 of `Chain-Support-Matrix.md` grades as
//! *"compiler never invoked"* — which is about tracing Aiken **source**.
//!
//! They also do **not** measure anything about `src/tracer.rs`. The
//! source-simulating interpreter is untouched: these tests neither route
//! through it nor assert anything about it. Replacing it is deliverable 2.
//!
//! # The corpus is reported, not curated
//!
//! `test_the_compiler_is_invoked_and_its_output_is_what_runs` reads **every**
//! `.ak` module in `test-programs/aiken/` and prints what happened to each,
//! including the modules the real compiler **refuses**, with the compiler's
//! own diagnostic. Five of the nineteen are refused as of this landing, and
//! that is a finding rather than a reason to narrow the population: a
//! source-simulated recorder that traces a module the compiler rejects is
//! precisely the §2.1 hazard this charter exists to close. The test asserts
//! the population size, so a corpus that shrank fails rather than reporting a
//! better ratio over fewer rows.
//!
//! # No skips, no network
//!
//! Every test here runs offline from committed inputs. There is no
//! live-chain arm, nothing is gated, and nothing is `#[ignore]`d — see
//! `tests/verify-cli-convention-no-silent-skip.sh`.

use std::path::{Path, PathBuf};

use codetracer_cardano_recorder::chain::ChainSource;
use codetracer_cardano_recorder::compile::{
    self, execute_artifact, execute_artifact_with_fault, ArtifactFault, CompiledArtifact,
    ProgramOutcome, CORPUS_COST_MODEL_EPOCH, CORPUS_DIR, CORPUS_PLUTUS_VERSION,
};
use codetracer_cardano_recorder::eras::cost_model_for;
use codetracer_cardano_recorder::fixture::FixtureSource;
use uplc::machine::cost_model::ExBudget;

/// The corpus the charter's reading is over.
const CORPUS_MODULES: usize = 19;

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CORPUS_DIR)
}

fn fixtures() -> FixtureSource {
    FixtureSource::at(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/koios"))
}

/// The language tag the corpus is compiled for.
fn language_tag() -> &'static str {
    use aiken_lang::plutus_version::PlutusVersion;
    match CORPUS_PLUTUS_VERSION {
        PlutusVersion::V1 => "PlutusV1",
        PlutusVersion::V2 => "PlutusV2",
        PlutusVersion::V3 => "PlutusV3",
    }
}

/// Compile one corpus module, the way the corpus reading does.
fn compile_one(module: &str) -> Vec<CompiledArtifact> {
    let deps = compile::corpus_library_modules(&corpus_dir())
        .expect("the corpus library directory must be readable");
    compile::compile_module(
        &corpus_dir().join(format!("{module}.ak")),
        &deps,
        CORPUS_PLUTUS_VERSION,
    )
    .unwrap_or_else(|e| panic!("{module}.ak must compile for this test to have a subject: {e:#}"))
}

// ---------------------------------------------------------------------------
// The deliverable's own entry
// ---------------------------------------------------------------------------

/// CXP-ADA-1 deliverable 3's verification entry.
///
/// The compiler is invoked on every corpus module; every artifact it produces
/// is encoded the way the chain stores a script, decoded back by the same two
/// lines `src/oracle.rs` uses on a chain-stored script, byte-compared — and
/// hash-compared — against the artifact the compiler produced, and then
/// stepped through the chain's CEK machine. The comparison is of the program
/// handed to the machine, in that order; see this file's header for why that
/// is not the same as comparing what the run did.
///
/// The control this entry names is driven red in
/// `test_a_deliberately_altered_script_produces_a_mismatch`, which is kept
/// separate so this test can report the population without the fault in it.
#[test]
fn test_the_compiler_is_invoked_and_its_output_is_what_runs() {
    let reading = compile::read_corpus(&corpus_dir(), &fixtures(), CORPUS_COST_MODEL_EPOCH)
        .expect("the corpus reading must run offline from committed inputs");

    print!("{}", compile::render(&reading));

    // The population is pinned. A corpus that shrank must fail here rather
    // than quietly report a better ratio over fewer rows.
    assert_eq!(
        reading.programs_total(),
        CORPUS_MODULES,
        "the corpus held {} module(s) and this reading is over {CORPUS_MODULES}",
        reading.programs_total(),
    );
    assert!(
        reading.artifacts_total() > 0,
        "no artifact was generated at all, so the comparison below would be vacuous"
    );

    // THE CLAUSE. Every artifact the compiler produced is the artifact that
    // ran: byte-identical in the chain's own storage encoding and carrying the
    // chain's own script hash for those bytes.
    let divergent = reading.divergent();
    assert!(
        divergent.is_empty(),
        "{} artifact(s) executed bytes that are NOT the compiler's output: {:?}",
        divergent.len(),
        divergent
            .iter()
            .map(|a| format!(
                "{}.{} compiler={} executed={}",
                a.artifact.module,
                a.artifact.test_name,
                a.artifact.script_hash,
                a.executed_script_hash
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        reading.artifacts_identical(),
        reading.artifacts_total(),
        "the identity must hold on every artifact, not most of them"
    );

    // And the machine really ran them. One corpus artifact is a program that
    // errors by construction — `error_paths_test.ak`'s `failing_path` calls a
    // function whose body is `fail`, which the compiler lowers to
    // `Term::Error` — so it reaches `EvaluationFailure` rather than `Done`.
    // That is the compiler's output behaving as the compiler's output, not a
    // recorder failure, so it is counted apart rather than excused.
    // Pinned in BOTH directions, and by name. `>=` was the first spelling and
    // it was one-sided: it tolerated *more* artifacts reaching `Done` than
    // expected, so the one corpus artifact that must fail could have stopped
    // failing — or every artifact could have been replaced by a constant —
    // without this test noticing. Driven red on review, 2026-10-31.
    let machine_failed: Vec<String> = reading
        .programs
        .iter()
        .filter_map(|p| match &p.outcome {
            ProgramOutcome::Compiled(rows) => Some(rows),
            ProgramOutcome::CompilerRefused(_) => None,
        })
        .flatten()
        .filter(|r| r.run.is_err())
        .map(|r| format!("{}.{}", r.artifact.module, r.artifact.test_name))
        .collect();
    assert_eq!(
        machine_failed,
        vec!["error_paths_test.failing_path".to_string()],
        "exactly one corpus artifact must fail in the machine, and it must be the one whose \
         source is `fail` — the compiler lowers that to Term::Error. Got: {machine_failed:?}"
    );
    assert_eq!(
        reading.artifacts_stepped() + 1,
        reading.artifacts_total(),
        "{} of {} artifact(s) stepped to Done; exactly one (the deliberately failing corpus \
         test) must not",
        reading.artifacts_stepped(),
        reading.artifacts_total(),
    );

    println!(
        "MEASURED: the real Aiken compiler (aiken-lang {}) was invoked on {} corpus module(s); \
         {} compiled and {} were refused by the compiler itself; {} artifact(s) were generated \
         and {} of {} executed the compiler's own bytes, byte-identical in the chain's storage \
         encoding and under the chain's own script hash; {} stepped to Done under epoch {} \
         (era {}) {} prices",
        aiken_lang::version::compiler_version(false),
        reading.programs_total(),
        reading.programs_compiled(),
        reading.programs_refused(),
        reading.artifacts_total(),
        reading.artifacts_identical(),
        reading.artifacts_total(),
        reading.artifacts_stepped(),
        reading.epoch,
        reading.era,
        reading.language_tag,
    );
}

// ---------------------------------------------------------------------------
// The control the entry names
// ---------------------------------------------------------------------------

/// *"Control: a deliberately altered script produces a mismatch, so the
/// comparison can fail in the direction the defect would take."*
///
/// Driven red on two independent alterations, because they fail the comparison
/// for different reasons and only together do they show it is a measurement:
///
/// 1. **The executed artifact is altered.** [`ArtifactFault`] rewrites the
///    program's version word *after* it is decoded and *before* it is
///    executed, so the machine runs something the compiler did not emit. The
///    altered program still decodes, still runs to `Done` and spends the
///    identical budget — so the byte comparison is the **only** thing that can
///    catch it, which is exactly the direction the defect would take. The
///    fault perturbs the artifact the recorder executes rather than the
///    comparison that consumes it; perturbing the comparison would prove only
///    that `!=` works.
/// 2. **The source is altered.** One character of a corpus module is changed
///    and the compiler re-invoked. The resulting artifact's bytes and hash both
///    differ from the unaltered module's, so the expectation is a function of
///    the program rather than a constant the comparison would match anyway.
#[test]
fn test_a_deliberately_altered_script_produces_a_mismatch() {
    let artifacts = compile_one("recursion_test");
    let artifact = artifacts.first().expect("recursion_test declares one test");

    let clean = execute_artifact(
        artifact,
        cost_model_for(
            &fixtures()
                .epoch_params(CORPUS_COST_MODEL_EPOCH)
                .expect("the committed capture must carry this epoch"),
            language_tag(),
        )
        .expect("the epoch must price this language"),
        ExBudget::max(),
    )
    .expect("the unaltered artifact must execute");
    assert!(
        clean.is_measured(),
        "the control needs a clean reading to be a control against"
    );

    // --- arm 1: the executed artifact is altered -------------------------
    let faulted = execute_artifact_with_fault(
        artifact,
        cost_model_for(
            &fixtures()
                .epoch_params(CORPUS_COST_MODEL_EPOCH)
                .expect("the committed capture must carry this epoch"),
            language_tag(),
        )
        .expect("the epoch must price this language"),
        ExBudget::max(),
        ArtifactFault::altered_version_word(),
    )
    .expect("the altered artifact still decodes and still runs; that is the point");

    assert!(
        !faulted.bytes_identical,
        "an altered script must not re-encode to the compiler's bytes — if it does, the byte \
         comparison is not comparing the artifact that ran"
    );
    assert!(
        !faulted.hash_identical,
        "an altered script must not carry the compiler's script hash"
    );
    assert!(
        !faulted.is_measured(),
        "an altered script must not read as measured"
    );

    // The alteration is caught by the byte comparison ALONE: the run still
    // succeeded and spent the identical budget, so nothing else could have.
    assert!(
        faulted.run.is_ok(),
        "the control must be caught by the comparison and not by the machine refusing to run \
         it; a decode or evaluation failure would prove the decoder works, not that the \
         comparison does"
    );
    assert_eq!(
        faulted.consumed(),
        clean.consumed(),
        "the altered artifact spent a different budget, so the budget arm could have caught it \
         and this control would not be measuring the byte comparison"
    );
    assert_eq!(
        faulted.step_count(),
        clean.step_count(),
        "the altered artifact took a different number of transitions, so the step count could \
         have caught it"
    );

    // --- arm 2: the source is altered ------------------------------------
    let original_path = corpus_dir().join("flow_test.ak");
    let original_src =
        std::fs::read_to_string(&original_path).expect("flow_test.ak must be readable");
    assert!(
        original_src.contains("compute() == 94"),
        "this control alters a known constant in flow_test.ak; the module changed shape"
    );
    let altered_src = original_src.replace("compute() == 94", "compute() == 95");

    let tmp = tempfile::tempdir().expect("a temporary directory");
    let altered_path = tmp.path().join("flow_test.ak");
    std::fs::write(&altered_path, &altered_src).expect("the altered copy must be writable");

    let unaltered = compile_one("flow_test");
    let altered = compile::compile_module(&altered_path, &[], CORPUS_PLUTUS_VERSION)
        .expect("the altered module is still valid Aiken and must compile");

    assert_ne!(
        altered[0].stored, unaltered[0].stored,
        "a one-character source change produced identical artifact bytes, so the comparison is \
         insensitive to the program and could not detect a substituted one"
    );
    assert_ne!(
        altered[0].script_hash, unaltered[0].script_hash,
        "a one-character source change produced an identical script hash"
    );

    println!(
        "CONTROL DRIVEN RED, two arms. (1) An altered artifact: rewriting the program's version \
         word after decoding and before execution leaves a program that still decodes, still \
         runs to Done in {} transitions and spends the identical budget (mem={} steps={}), and \
         the byte comparison is the ONLY arm that catches it — executed hash {} vs the \
         compiler's {}. (2) An altered source: changing one character of flow_test.ak moves the \
         artifact from {} bytes/hash {} to {} bytes/hash {}, so the expectation is a function of \
         the program and not a constant",
        faulted.step_count().expect("the faulted run completed"),
        faulted.consumed().expect("the faulted run completed").mem,
        faulted.consumed().expect("the faulted run completed").cpu,
        faulted.executed_script_hash,
        faulted.artifact.script_hash,
        unaltered[0].stored.len(),
        unaltered[0].script_hash,
        altered[0].stored.len(),
        altered[0].script_hash,
    );
}

// ---------------------------------------------------------------------------
// There is no second, looser front-end
// ---------------------------------------------------------------------------

/// A module the real compiler rejects is **reported with the compiler's own
/// diagnostic** and is not compiled by anything else.
///
/// This is the control on the deliverable's own premise. A recorder that
/// accepted a module the compiler rejects would be back to having a front-end
/// of its own, which is the defect §2.2 grades — so the absence of a fallback
/// has to be measured rather than asserted in a comment.
///
/// Driven red on an ill-typed module written for this test, and then confirmed
/// on the corpus itself, where five modules are refused in the standing
/// reading.
#[test]
fn test_a_module_the_compiler_rejects_is_reported_and_not_compiled_by_us() {
    let tmp = tempfile::tempdir().expect("a temporary directory");

    // Ill-typed on purpose: `Bool + Int` is not an expression any Aiken
    // front-end should accept, and the recorder has no business having an
    // opinion about it.
    let path = tmp.path().join("ill_typed.ak");
    std::fs::write(
        &path,
        "fn broken() -> Int {\n  let x = True\n  x + 1\n}\n\ntest t() {\n  broken() == 1\n}\n",
    )
    .expect("the module must be writable");

    let err = compile::compile_module(&path, &[], CORPUS_PLUTUS_VERSION)
        .expect_err("an ill-typed module must be refused, not compiled");
    let message = format!("{err:#}");
    assert!(
        message.contains("the Aiken type checker refused this module"),
        "the refusal must come from the compiler's type checker; got: {message}"
    );
    assert!(
        message.contains("CouldNotUnify") || message.contains("unify"),
        "the refusal must carry the compiler's own diagnostic rather than a message of ours; \
         got: {message}"
    );

    // And on the corpus: the refusals are reported as refusals, each with a
    // reason, rather than appearing as artifacts.
    let reading = compile::read_corpus(&corpus_dir(), &fixtures(), CORPUS_COST_MODEL_EPOCH)
        .expect("the corpus reading must run");
    let refused: Vec<(&str, &str)> = reading
        .programs
        .iter()
        .filter_map(|p| match &p.outcome {
            ProgramOutcome::CompilerRefused(why) => Some((p.module.as_str(), why.as_str())),
            ProgramOutcome::Compiled(_) => None,
        })
        .collect();
    assert!(
        !refused.is_empty(),
        "no corpus module was refused, so this arm measured nothing. If the corpus was fixed, \
         that is good news and this assertion is the thing that says so out loud"
    );
    for (module, why) in &refused {
        assert!(
            why.contains("the Aiken parser refused this module")
                || why.contains("the Aiken type checker refused this module"),
            "{module}'s refusal does not name the compiler stage that refused it: {why}"
        );
    }

    println!(
        "CONTROL DRIVEN RED: an ill-typed module is refused by the compiler's own type checker \
         and produces no artifact. On the corpus, {} of {} module(s) are refused and each \
         refusal names the compiler stage and the construct: {}",
        refused.len(),
        reading.programs_total(),
        refused
            .iter()
            .map(|(m, w)| format!("{m}: {w}"))
            .collect::<Vec<_>>()
            .join(" | "),
    );
}

// ---------------------------------------------------------------------------
// The cost model is an era's, and a refusal stays a refusal
// ---------------------------------------------------------------------------

/// The corpus is charged under a **named epoch's own prices**, and an epoch
/// that cannot price the language refuses rather than approximating.
///
/// `CostModel::default()` was removed from `src/` by the deliverable-4
/// landing, and it is not reintroduced by this one. A local `.ak` module
/// settled in no epoch, so there is no epoch to read off it — but that is a
/// reason to **name** one and print it, not a licence to charge prices no era
/// ever set.
///
/// The control is an epoch whose parameters carry no model for this language:
/// Alonzo's epoch 290 served PlutusV1 only. The corpus reading must fail
/// loudly there rather than silently fall back.
#[test]
fn test_the_corpus_is_charged_under_a_named_epochs_own_prices() {
    let src = fixtures();

    let params = src
        .epoch_params(CORPUS_COST_MODEL_EPOCH)
        .expect("the committed capture must carry the corpus epoch");
    let served = params
        .cost_models
        .get(language_tag())
        .expect("epoch 664 serves a PlutusV3 cost model")
        .len();
    assert!(
        cost_model_for(&params, language_tag()).is_ok(),
        "the corpus epoch must price the corpus language"
    );

    // CONTROL: an epoch that does not price this language. The reading must
    // not be produced at all.
    let alonzo = src
        .epoch_params(290)
        .expect("the committed capture must carry epoch 290");
    assert!(
        !alonzo.cost_models.contains_key(language_tag()),
        "this control needs an epoch with no {} model; epoch 290 gained one",
        language_tag()
    );
    let refusal = compile::read_corpus(&corpus_dir(), &src, 290)
        .expect_err("an epoch that cannot price the language must refuse the whole reading");
    let refusal = format!("{refusal:#}");
    assert!(
        refusal.contains("carries no PlutusV3 cost model"),
        "the refusal must say which model is missing; got: {refusal}"
    );

    println!(
        "MEASURED: the corpus is charged under epoch {} (era {}), which serves {served} {} \
         parameter(s). CONTROL DRIVEN RED: charging it under epoch 290, which served PlutusV1 \
         only, refuses the whole reading rather than approximating — \"{}\"",
        CORPUS_COST_MODEL_EPOCH,
        params.era.as_deref().unwrap_or("unknown"),
        language_tag(),
        refusal.lines().next().unwrap_or_default(),
    );
}

// ---------------------------------------------------------------------------
// The artifact's identity is the chain's, recomputed here
// ---------------------------------------------------------------------------

/// The stored artifact is the blueprint's own encoding, and its name is the
/// chain's own hash — recomputed in this test rather than trusted.
///
/// Three independent recomputations, following the pattern
/// `tests/fixtures/koios/README.md` establishes for a capture's identity:
///
/// - the stored bytes are CBOR-wrapped flat UPLC: they decode through the same
///   call `src/oracle.rs` makes on a chain-stored script;
/// - the decoded program re-encodes to exactly those bytes;
/// - the script hash is blake2b-224 over the language-tagged script bytes,
///   recomputed here with `pallas_crypto` rather than read back from the value
///   that produced it.
///
/// Nothing is committed as an expected-UPLC fixture, so there is no derived
/// artifact to go stale: every expectation in this file is produced by a fresh
/// compile in the same run.
#[test]
fn test_the_compiled_artifact_carries_the_chains_own_identity() {
    use pallas_crypto::hash::Hasher;

    let artifacts = compile_one("pattern_match_test");
    let artifact = artifacts.first().expect("one test in the module");

    // 1. It decodes the way a chain-stored script does.
    let program = artifact
        .decode()
        .expect("the compiler's output must decode as flat UPLC");

    // 2. And re-encodes to exactly the same bytes.
    let round_trip = uplc::ast::Program::<uplc::ast::DeBruijn>::from(program.clone());
    let re_encoded = round_trip
        .to_cbor()
        .expect("the decoded program must re-encode");
    assert_eq!(
        re_encoded, artifact.stored,
        "the compiler's artifact does not survive the chain's own encode/decode round trip, so \
         'byte-identical' would be measuring the encoder rather than the artifact"
    );

    // 3. The name is the chain's, recomputed independently.
    let tag: u8 = match artifact.language {
        pallas_primitives::conway::Language::PlutusV1 => 1,
        pallas_primitives::conway::Language::PlutusV2 => 2,
        pallas_primitives::conway::Language::PlutusV3 => 3,
    };
    let mut hasher = Hasher::<224>::new();
    hasher.input(&[tag]);
    hasher.input(&artifact.stored);
    let recomputed = hasher.finalize().to_string();
    assert_eq!(
        recomputed, artifact.script_hash,
        "the script hash this module reports is not blake2b-224 over the language-tagged script \
         bytes, which is how the chain names a script"
    );

    println!(
        "MEASURED: {}.{}'s artifact is {} byte(s) of CBOR-wrapped flat UPLC, decodes through the \
         same call src/oracle.rs makes on a chain-stored script, re-encodes to the identical \
         bytes, and its name {} is blake2b-224 over 0x{:02x} || those bytes — recomputed here \
         with pallas_crypto rather than read back",
        artifact.module,
        artifact.test_name,
        artifact.stored.len(),
        recomputed,
        tag,
    );
}

// ---------------------------------------------------------------------------
// What the compiler's output is, and what it is not
// ---------------------------------------------------------------------------

/// The compiler constant-folds a nullary `test`, and the consequence is
/// recorded rather than glossed.
///
/// This is not a defect and it is not a failure of the deliverable's clause —
/// the artifact executed is still exactly the artifact the compiler produced.
/// It is a **measured limit on what that artifact can be traced to**, and it
/// belongs in the record because it bears directly on deliverable 2: a step
/// stream taken from the compiler's output for one of these modules cannot
/// show the source's intermediate values, because the compiler computed them
/// at compile time and emitted the answer.
#[test]
fn test_a_constant_folded_test_compiles_to_its_answer_and_the_consequence_is_stated() {
    let reading = compile::read_corpus(&corpus_dir(), &fixtures(), CORPUS_COST_MODEL_EPOCH)
        .expect("the corpus reading must run");

    let mut folded: Vec<(String, String, usize, usize)> = Vec::new();
    let mut unfolded: Vec<(String, String, usize, usize)> = Vec::new();
    for program in &reading.programs {
        if let ProgramOutcome::Compiled(rows) = &program.outcome {
            for row in rows {
                let steps = row.step_count().unwrap_or(0);
                let entry = (
                    program.module.clone(),
                    row.artifact.test_name.clone(),
                    row.artifact.stored.len(),
                    steps,
                );
                // A program the compiler reduced to a single constant is three
                // transitions: Compute, Return, Done.
                if row.run.is_ok() && steps <= 3 {
                    folded.push(entry);
                } else {
                    unfolded.push(entry);
                }
            }
        }
    }

    assert!(
        !folded.is_empty(),
        "no corpus artifact was constant-folded, so this reading has no subject. If the \
         compiler stopped folding, the consequence recorded here is gone and should be removed \
         from the record rather than left standing"
    );
    assert!(
        !unfolded.is_empty(),
        "every corpus artifact was constant-folded, which would mean the stepping path has no \
         real program to observe anywhere in the corpus"
    );

    // The folded ones collapse onto one artifact identity, because they all
    // compile to the same constant. That is worth asserting: it is why the
    // script hash cannot be used as a per-module identity for this corpus.
    let folded_hashes: std::collections::BTreeSet<String> = reading
        .programs
        .iter()
        .filter_map(|p| match &p.outcome {
            ProgramOutcome::Compiled(rows) => Some(rows),
            ProgramOutcome::CompilerRefused(_) => None,
        })
        .flatten()
        .filter(|r| r.run.is_ok() && r.step_count().unwrap_or(0) <= 3)
        .map(|r| r.artifact.script_hash.clone())
        .collect();

    println!(
        "MEASURED: {} of {} corpus artifact(s) are constant-folded by the real compiler to a \
         single constant — {} CEK transitions, {} distinct script hash(es) across all of them, \
         because they all compile to the same value. The other {} carry a real program ({} \
         transitions at the largest). CONSEQUENCE, recorded for deliverable 2: for a folded \
         module the compiler's artifact contains none of the source's intermediate values, so a \
         step stream taken from it cannot show them. The artifact executed is still exactly the \
         artifact the compiler produced; what is bounded is what can be read off it. Folded: {:?}",
        folded.len(),
        folded.len() + unfolded.len(),
        folded.iter().map(|f| f.3).max().unwrap_or(0),
        folded_hashes.len(),
        unfolded.len(),
        unfolded.iter().map(|f| f.3).max().unwrap_or(0),
        folded
            .iter()
            .map(|(m, t, b, s)| format!("{m}.{t} {b}B {s} steps"))
            .collect::<Vec<_>>(),
    );
}
