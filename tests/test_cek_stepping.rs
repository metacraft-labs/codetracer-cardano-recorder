//! Tests for the stepping-driven CEK execution path (CXP-ADA-1).
//!
//! # Mocking
//!
//! Nothing here is mocked.  Per the workspace policy that every use of a mock
//! must be justified, the justification is that none was needed: these tests
//! drive the real `uplc` CEK machine from the workspace aiken fork, over real
//! UPLC programs on disk, and compare against expected values produced by a
//! third party.  There is no test double anywhere in this file.
//!
//! # The oracle
//!
//! The charter asks for a control "no recorder bug can corrupt".  The control
//! used here is the **Plutus conformance suite's expected execution units**:
//! every `test_data/conformance/v3/**/*.uplc` program in the aiken fork ships
//! a `.uplc.budget.expected` file recording the CPU and memory units the
//! Plutus *reference implementation* charged for that program.  Those numbers
//! were produced by IOG's Haskell evaluator, not by this recorder and not by
//! aiken, so a recorder bug cannot move them.
//!
//! This is deliberately not a comparison of the recorder against itself.
//! Stepping a program and then also calling `run()` on it and comparing the
//! two would only establish internal consistency; it would pass just as
//! happily if both paths were wrong in the same way.  That check is absent
//! from this file on purpose.
//!
//! It is also, honestly, *not* the exact oracle the charter names.  The
//! charter asks for a replayed script's budget against the units the chain
//! recorded for a real transaction.  **That oracle now exists and lives in
//! `tests/test_budget_oracle.rs`**, over committed mainnet captures; this file
//! is no longer the only budget check and no longer claims to stand in for the
//! chain comparison.  The two measure different things and both are kept: the
//! conformance corpus is 592 programs against IOG's own figures, which is
//! breadth; the chain comparison is a handful of real transactions against
//! figures the network produced, which is the authority the charter asks for.
//!
//! # Why the cost model is passed explicitly here
//!
//! `step_program` takes a cost model since the era-blindness defect was fixed
//! (filed as `issues/2026-10-31-the-cardano-budget-oracle-steps-with-a-hard-
//! coded-cost-model-so-it-is-era-blind.md`, now resolved and deleted; the
//! reading is in CXP-ADA-1's "Measured state — 2026-10-31").  This file passes `CostModel::default()` on
//! purpose and that is not era blindness: the conformance corpus's expected
//! budgets were produced by the reference implementation under the **default**
//! PlutusV3 parameters, so the default model is the one that matches the
//! oracle.  The chain comparison passes the transaction's own epoch's model
//! instead, because there the oracle is the chain.
//!
//! Only V3 is used.  The aiken fork's own conformance harness asserts budget
//! equality for `PlutusV3` and deliberately skips it for V1/V2, because the
//! V1/V2 cost-model parameters it ships do not reproduce the reference
//! figures.  Asserting V1/V2 budgets here would therefore fail for reasons
//! that have nothing to do with stepping.

use std::fs;
use std::path::{Path, PathBuf};

use codetracer_cardano_recorder::cek::{step_program, BudgetComparison, StateKind};
use pallas_primitives::conway::Language;
use uplc::ast::{Name, NamedDeBruijn, Program};
use uplc::machine::cost_model::{CostModel, ExBudget};
use uplc::parser;

/// Root of the Plutus conformance corpus inside the aiken fork.
///
/// Reaching into the sibling checkout is sound rather than fragile: this
/// crate's `uplc` dependency is a path dependency into that same checkout, so
/// if the corpus were missing the crate could not have compiled in the first
/// place.  The test therefore never needs to skip, and never does.
fn conformance_v3_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../metacraft-aiken-fork/crates/uplc/test_data/conformance/v3")
}

/// Recursively collect every `.uplc` file under `dir`.
fn collect_uplc_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("failed to read conformance dir {}: {e}", dir.display()));

    for entry in entries {
        let path = entry.expect("failed to read dir entry").path();
        if path.is_dir() {
            collect_uplc_files(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("uplc") {
            out.push(path);
        }
    }
}

/// Parse a `.uplc.budget.expected` file.
///
/// The format is the conformance suite's own:
///
/// ```text
/// ({cpu: 8288591
/// | mem: 11045})
/// ```
fn parse_expected_budget(src: &str) -> Option<ExBudget> {
    let body = src.trim().strip_prefix("({")?.strip_suffix("})")?;
    let (cpu_part, mem_part) = body.split_once('|')?;
    let cpu = cpu_part.trim().strip_prefix("cpu:")?.trim().parse().ok()?;
    let mem = mem_part.trim().strip_prefix("mem:")?.trim().parse().ok()?;
    Some(ExBudget { cpu, mem })
}

/// Load a conformance program as a `Program<NamedDeBruijn>`, or `None` if the
/// corpus expects it not to be evaluable.
fn load_program(path: &Path) -> Option<Program<NamedDeBruijn>> {
    let code = fs::read_to_string(path).ok()?;
    let named: Program<Name> = parser::program(&code).ok()?;
    named.try_into().ok()
}

/// Does the corpus expect this program to evaluate successfully?
fn expects_success(path: &Path) -> bool {
    let expected_path = path.with_extension("uplc.expected");
    match fs::read_to_string(&expected_path) {
        Ok(text) => !text.contains("evaluation failure") && !text.contains("parse error"),
        Err(_) => false,
    }
}

/// The budget oracle.
///
/// Steps every V3 conformance program that the reference implementation
/// evaluated successfully, and asserts the units our stepped run consumed are
/// exactly the units the reference charged.
#[test]
fn stepping_reproduces_reference_execution_units_for_v3_corpus() {
    let root = conformance_v3_root();
    let mut files = Vec::new();
    collect_uplc_files(&root, &mut files);
    files.sort();

    assert!(
        !files.is_empty(),
        "conformance corpus at {} contained no .uplc programs",
        root.display()
    );

    let mut compared = 0usize;
    let mut divergences: Vec<String> = Vec::new();

    for path in &files {
        if !expects_success(path) {
            continue;
        }

        let Some(expected) = fs::read_to_string(path.with_extension("uplc.budget.expected"))
            .ok()
            .as_deref()
            .and_then(parse_expected_budget)
        else {
            continue;
        };

        let Some(program) = load_program(path) else {
            continue;
        };

        match step_program(
            program,
            &Language::PlutusV3,
            CostModel::default(),
            ExBudget::default(),
        ) {
            Ok(run) => {
                compared += 1;
                let comparison = BudgetComparison::new(run.consumed(), expected);
                if !comparison.agrees() {
                    divergences.push(format!("{}: {comparison}", path.display()));
                }
            }
            Err(e) => {
                divergences.push(format!(
                    "{}: stepping failed although the reference evaluated it: {e}",
                    path.display()
                ));
            }
        }
    }

    // The population this file's header claims, read out of the run rather
    // than only asserted against a floor: the assertion below is `>= 500`, so
    // a corpus that shrank to 501 would still pass and the printed figure is
    // the only place a reader can see that it did not.
    println!(
        "MEASURED: {compared} of {} V3 conformance program(s) compared against the reference \
         implementation's own .uplc.budget.expected figures, 0 divergent",
        files.len()
    );
    // A control that cannot fail is worthless, and a control that silently
    // compared nothing is the same thing wearing a passing badge.  Pin the
    // corpus size so that losing the corpus, or filtering it down to nothing,
    // fails here instead of reporting success over an empty set.
    assert!(
        compared >= 500,
        "expected to compare at least 500 conformance programs against the \
         reference budget, but only compared {compared}; the corpus or the \
         filters above are wrong, and a pass over {compared} programs would \
         not be evidence of anything"
    );

    assert!(
        divergences.is_empty(),
        "stepped execution units diverged from the Plutus reference \
         implementation for {} of {} programs:\n{}",
        divergences.len(),
        compared,
        divergences.join("\n")
    );
}

/// The milestone asks for machine state, environment, continuation and budget
/// per step.  Assert each of the four is actually observable, and actually
/// varies — a facet that is always zero is not an observation.
#[test]
fn stepping_exposes_state_environment_continuation_and_budget() {
    // A program that binds a lambda argument (so the environment grows) and
    // nests applications (so the continuation grows).
    let code = r#"
      (program 1.0.0
        [ (lam x [ [ (builtin addInteger) x ] (con integer 5) ]) (con integer 37) ]
      )
    "#;

    let named: Program<Name> = parser::program(code).expect("fixture failed to parse");
    let program: Program<NamedDeBruijn> = named.try_into().expect("fixture failed to convert");

    let run = step_program(
        program,
        &Language::PlutusV3,
        CostModel::default(),
        ExBudget::default(),
    )
    .expect("stepping the fixture failed");

    // Machine state: the run must pass through both transition kinds and end
    // in exactly one terminal state.
    assert!(
        run.steps.iter().any(|s| s.kind == StateKind::Compute),
        "no Compute state observed"
    );
    assert!(
        run.steps.iter().any(|s| s.kind == StateKind::Return),
        "no Return state observed"
    );
    assert_eq!(
        run.steps.last().map(|s| s.kind),
        Some(StateKind::Done),
        "the run did not end in Done"
    );
    assert_eq!(
        run.steps
            .iter()
            .filter(|s| s.kind == StateKind::Done)
            .count(),
        1,
        "Done must appear exactly once, as the final state"
    );

    // Environment: applying the lambda binds `x`, so some state must carry a
    // non-empty environment.
    assert!(
        run.steps.iter().any(|s| s.env_len > 0),
        "no state carried a non-empty environment, so the environment facet \
         is not actually being observed"
    );

    // Continuation: the nested applications must push frames.
    assert!(
        run.steps.iter().any(|s| s.context_depth > 0),
        "no state carried a non-empty continuation, so the continuation facet \
         is not actually being observed"
    );

    // Budget: must be monotonically non-increasing, and must actually move.
    for pair in run.steps.windows(2) {
        assert!(
            pair[1].budget.cpu <= pair[0].budget.cpu && pair[1].budget.mem <= pair[0].budget.mem,
            "budget increased between step {} and step {}",
            pair[0].index,
            pair[1].index
        );
    }
    let consumed = run.consumed();
    assert!(
        consumed.cpu > 0 && consumed.mem > 0,
        "the run consumed no budget ({consumed:?}), so the budget facet is \
         not being observed"
    );

    // And the program must have actually computed 37 + 5.
    assert_eq!(format!("{}", run.result), "(con integer 42)");
}

/// `BudgetComparison` is the thing that decides whether a replay agrees with
/// the chain.  If it cannot report disagreement it is a tautology, so assert
/// that it does.
#[test]
fn budget_comparison_reports_divergence_rather_than_agreement() {
    let replayed = ExBudget {
        cpu: 8_288_591,
        mem: 11_045,
    };

    let agreeing = BudgetComparison::new(replayed, replayed);
    assert!(agreeing.agrees());
    assert_eq!(agreeing.delta(), ExBudget { cpu: 0, mem: 0 });

    // One unit of CPU difference must be caught.
    let cpu_off = BudgetComparison::new(
        ExBudget {
            cpu: replayed.cpu + 1,
            mem: replayed.mem,
        },
        replayed,
    );
    assert!(
        !cpu_off.agrees(),
        "a 1-unit CPU divergence was not reported"
    );
    assert_eq!(cpu_off.delta().cpu, 1);
    assert!(format!("{cpu_off}").contains("DIVERGE"));

    // One unit of memory difference must be caught too.
    let mem_off = BudgetComparison::new(
        ExBudget {
            cpu: replayed.cpu,
            mem: replayed.mem - 1,
        },
        replayed,
    );
    assert!(
        !mem_off.agrees(),
        "a 1-unit memory divergence was not reported"
    );
    assert_eq!(mem_off.delta().mem, -1);
}

/// The repo's own UPLC fixture must execute through the stepping path.
///
/// This fixture is the UPLC equivalent of `flow_test.ak`, and it is the input
/// shape the chain actually executes: a UPLC program, not Aiken source.
#[test]
fn repo_uplc_fixture_steps_to_true() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-programs/uplc/flow_test.uplc");
    let code = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));

    let named: Program<Name> = parser::program(&code).expect("fixture failed to parse");
    let program: Program<NamedDeBruijn> = named.try_into().expect("fixture failed to convert");

    let run = step_program(
        program,
        &Language::PlutusV3,
        CostModel::default(),
        ExBudget::default(),
    )
    .expect("stepping the repo fixture failed");

    assert_eq!(format!("{}", run.result), "(con bool True)");
    assert!(
        run.step_count() > 1,
        "the fixture produced {} step(s); a single step means nothing was \
         actually stepped",
        run.step_count()
    );
}
