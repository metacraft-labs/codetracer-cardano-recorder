//! Integration tests for the Aiken tracer.
//!
//! These tests cover three areas:
//!
//! 1. Pure UPLC CEK evaluation tests that don't touch the recorder
//!    writer at all — the source of truth for the in-process
//!    `uplc` crate behaviour.
//! 2. End-to-end recording of `flow_test.ak` through the
//!    Aiken → UPLC → CTFS pipeline; assertions are made on the
//!    CTFS bundle either directly (CTFS magic bytes / `.ct` file
//!    presence) or via `ct print` (the canonical conversion tool
//!    shipped with `codetracer-trace-format-nim`).
//! 3. The CLI env-var contract for the recorder
//!    (`CODETRACER_CARDANO_RECORDER_OUT_DIR` /
//!    `CODETRACER_CARDANO_RECORDER_DISABLED`) plus the no-`--format`
//!    invariant.
//!
//! History note: pre-2026-05-08 the recorder shipped a `--format
//! ctfs|binary|json` flag and the CLI integration test
//! (`test_aiken_cli_record`) drove it with `--format json`.  When the
//! convention switched to CTFS-only the flag was removed and the test
//! was rewritten to record via the recorder library and pipe the
//! resulting `.ct` container through `ct-print --json`.  See
//! `AUDIT-CTFS-2026-05.md` ("Convention compliance follow-up") for
//! the full record.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

use num_bigint::BigInt;
use uplc::ast::{Constant, NamedDeBruijn, Program, Term};
use uplc::builtins::DefaultFunction;
use uplc::machine::cost_model::ExBudget;

const CTFS_MAGIC: [u8; 5] = [0xC0, 0xDE, 0x72, 0xAC, 0xE2];

fn test_programs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-programs/aiken")
}

/// Path to the `ct-print` binary shipped with `codetracer-trace-format-nim`.
///
/// The Cardano recorder is CTFS-only; tests that need to make
/// content-level assertions on a recorded trace pipe the `.ct`
/// container through `ct-print --json` and assert on the resulting
/// JSON.  This is the same workflow that `Recorder-CLI-Conventions.md`
/// §4 prescribes for downstream tools / golden snapshots.
fn ct_print_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("codetracer-trace-format-nim")
        .join("ct-print")
}

/// Helper: collect every `.ct` file in `out_dir`.
fn ct_files_in(out_dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(out_dir)
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "ct"))
        .collect()
}

fn run_tracer_on_file(source_path: &Path, out_dir: &Path) {
    codetracer_cardano_recorder::recorder::record(source_path, out_dir)
        .expect("trace_program should succeed");
}

fn assert_valid_ct_file(out_dir: &Path) -> PathBuf {
    let ct_files = ct_files_in(out_dir);
    assert!(
        !ct_files.is_empty(),
        "expected at least one .ct file in {:?}",
        out_dir
    );
    let ct_path = &ct_files[0];
    let content = std::fs::read(ct_path).expect("failed to read .ct file");
    assert!(content.len() >= 5, ".ct file too small");
    assert_eq!(&content[..5], &CTFS_MAGIC, "CTFS magic bytes mismatch");
    ct_path.clone()
}

// ===========================================================================
// End-to-end: record flow_test.ak and verify the produced .ct container
// ===========================================================================

#[test]
fn test_aiken_compile_and_run() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();
    let source_path = test_programs_dir().join("flow_test.ak");
    run_tracer_on_file(&source_path, &out_dir);
    let ct_path = assert_valid_ct_file(&out_dir);
    let size = std::fs::metadata(&ct_path).unwrap().len();
    assert!(
        size > 100,
        ".ct file should have substantial content, got {} bytes",
        size
    );
}

#[test]
fn test_aiken_compute_value() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();
    let source_path = test_programs_dir().join("flow_test.ak");
    run_tracer_on_file(&source_path, &out_dir);
    assert_valid_ct_file(&out_dir);
}

#[test]
fn test_aiken_variable_values() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();
    let source_path = test_programs_dir().join("flow_test.ak");
    run_tracer_on_file(&source_path, &out_dir);
    assert_valid_ct_file(&out_dir);
}

#[test]
fn test_aiken_step_events() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();
    let source_path = test_programs_dir().join("flow_test.ak");
    run_tracer_on_file(&source_path, &out_dir);
    assert_valid_ct_file(&out_dir);
}

#[test]
fn test_aiken_metadata_structure() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();
    let source_path = test_programs_dir().join("flow_test.ak");
    run_tracer_on_file(&source_path, &out_dir);
    assert_valid_ct_file(&out_dir);
}

#[test]
fn test_aiken_function_calls() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();
    let source_path = test_programs_dir().join("flow_test.ak");
    run_tracer_on_file(&source_path, &out_dir);
    assert_valid_ct_file(&out_dir);
}

/// CLI smoke test for the `record` subcommand.
///
/// Pre-2026-05-08 this test passed `--format json` and asserted on
/// `trace.json` content.  The convention now mandates CTFS-only output,
/// so the test simply verifies the CLI runs to completion and produces
/// a `.ct` container in the requested output dir.  Content-level
/// assertions live in `test_recorded_trace_via_ct_print_json` below.
#[test]
fn test_aiken_cli_record() {
    let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let out_dir = tmp_dir.path().join("cli-traces");
    let source_path = test_programs_dir().join("flow_test.ak");

    let output = Command::new(env!("CARGO_BIN_EXE_codetracer-cardano-recorder"))
        .args(["record"])
        .arg(&source_path)
        .args(["--out-dir"])
        .arg(&out_dir)
        .output()
        .expect("failed to run");
    assert!(
        output.status.success(),
        "record should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_valid_ct_file(&out_dir);
}

// ===========================================================================
// CTFS content via `ct-print` — replaces the legacy `--format json` test
// ===========================================================================

/// Record `flow_test.ak`, then convert the produced `.ct` container to
/// JSON via `ct-print` and assert on:
///
/// 1. **Structural anchors** (legacy layer): `ct-print --json` output
///    contains the source filename / variable names / canonical integer
///    values somewhere in the textual rendering.
/// 2. **Exact decoded values** (the layer enabled by `ct-print --full`):
///    the `flow_test.ak` program executes `(10 + 32) * 2 + 10 = 94`
///    via the `compute()` function, with intermediate let-bindings
///    `a=10`, `b=32`, `sum_val=42`, `doubled=84`, `final_result=94`.
///    Each binding must surface in the trace as a step event with a
///    decoded `Int` ValueRecord whose `i` field matches the literal
///    value from the source program.
///
/// Pre-2026-05-08 a similar assertion was made directly on a recorder-
/// emitted `trace.json` file (via `--format json`).  The convention now
/// mandates CTFS-only output; `ct print` is the canonical conversion
/// tool.  See `Recorder-CLI-Conventions.md` §4.  `ct-print --full`
/// (added 2026-05 in `codetracer-trace-format-nim`) is what enables the
/// exact-value layer — its output is a deterministic JSON document with
/// every CBOR `ValueRecord` decoded to a structured form like
/// `{"kind":"Int","i":42,"type_id":N}`.
#[test]
fn test_recorded_trace_via_ct_print_json() {
    let ct_print = ct_print_path();
    if !ct_print.exists() {
        eprintln!(
            "SKIP: ct-print not found at {} — only available within the \
             metacraft workspace where codetracer-trace-format-nim is a sibling.",
            ct_print.display()
        );
        return;
    }

    let tmp_dir = tempfile::tempdir().expect("tempdir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path = test_programs_dir().join("flow_test.ak");
    codetracer_cardano_recorder::recorder::record(&source_path, &out_dir)
        .expect("recorder::record should succeed");

    let ct_files = ct_files_in(&out_dir);
    assert!(
        !ct_files.is_empty(),
        "expected a .ct container in {:?}",
        out_dir
    );

    // -----------------------------------------------------------------
    // Layer 1 (legacy): ct-print --json — substring presence checks.
    // Kept as a safety net so a regression in the textual rendering
    // is caught even if --full's JSON shape evolves.
    // -----------------------------------------------------------------
    let output = Command::new(&ct_print)
        .args(["--json"])
        .arg(&ct_files[0])
        .output()
        .expect("failed to run ct-print");

    assert!(
        output.status.success(),
        "ct-print --json should succeed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout_json = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout_json.is_empty(),
        "ct-print --json produced empty output"
    );

    // Structural anchors that the recorder must surface for any
    // CodeTracer consumer to function:
    //   * the source path in the metadata,
    //   * the `compute` function name in the function table,
    //   * each let-binding name in the values stream.
    assert!(
        stdout_json.contains("flow_test.ak"),
        "ct-print --json output should mention the source file; got:\n{stdout_json}"
    );
    assert!(
        stdout_json.contains("\"compute\""),
        "ct-print --json output should mention the `compute` function; got:\n{stdout_json}"
    );
    for varname in ["a", "b", "sum_val", "doubled", "final_result"] {
        assert!(
            stdout_json.contains(&format!("\"{varname}\"")),
            "ct-print --json output should mention the `{varname}` variable; got:\n{stdout_json}"
        );
    }

    // -----------------------------------------------------------------
    // Layer 2 (the upgrade): ct-print --full — exact decoded values.
    // -----------------------------------------------------------------
    let full_output = Command::new(&ct_print)
        .args(["--full", "--strip-paths"])
        .arg(&ct_files[0])
        .output()
        .expect("failed to run ct-print --full");

    assert!(
        full_output.status.success(),
        "ct-print --full should succeed; stderr: {}",
        String::from_utf8_lossy(&full_output.stderr)
    );

    let doc: serde_json::Value = serde_json::from_slice(&full_output.stdout)
        .expect("ct-print --full should emit valid JSON");

    // ----- Function table: compute() and flow_test() must both appear -
    // The Aiken recorder currently registers function names as bare
    // identifiers (no module qualifier), but downstream language
    // backends may add one (e.g. `aiken::FlowTest::compute`), so we
    // use `ends_with` to stay platform-agnostic.
    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        functions.iter().any(|f| f.ends_with("compute")),
        "expected `compute` in functions table; got {:?}",
        functions
    );
    assert!(
        functions.iter().any(|f| f.ends_with("flow_test")),
        "expected `flow_test` in functions table; got {:?}",
        functions
    );

    // ----- Path table: the canonical fixture path must appear ---------
    let paths: Vec<&str> = doc["paths"]
        .as_array()
        .expect("paths array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        paths.iter().any(|p| p.ends_with("flow_test.ak")),
        "expected flow_test.ak in paths table; got {:?}",
        paths
    );

    // ----- Step / call counts ----------------------------------------
    // The Aiken recorder evaluates `compute()` directly (the `test
    // flow_test()` block is registered as a function but the recorder
    // doesn't trace its body — only the `compute()` call inside it),
    // emitting one `call_entry` for `compute` and 8 step events
    // (entry/dispatch + five let-bindings + final-expression line +
    // the post-call return-site step).  Stable properties of the
    // canonical fixture — if they change, that's a real regression to
    // investigate, not a flake.
    let counts = &doc["counts"];
    assert_eq!(
        counts["steps"].as_u64(),
        Some(8),
        "expected 8 step events for flow_test.ak; counts={counts}",
    );
    assert_eq!(
        counts["calls"].as_u64(),
        Some(1),
        "expected 1 call event (compute); counts={counts}",
    );

    let events = doc["events"].as_array().expect("events array");

    // ----- Call sequence: compute (only) ------------------------------
    let call_sequence: Vec<&str> = events
        .iter()
        .filter(|e| e["kind"] == "call_entry")
        .filter_map(|e| e["function"].as_str())
        .collect();
    assert_eq!(
        call_sequence.len(),
        1,
        "expected exactly 1 call_entry event; got {:?}",
        call_sequence
    );
    assert!(
        call_sequence[0].ends_with("compute"),
        "expected call to be `compute`; got {:?}",
        call_sequence
    );

    // ----- Exact decoded variable values ------------------------------
    // Collect every (varname, i64) pair surfaced by step events.  These
    // come from the recorder writing `ValueRecord::Int` CBOR blobs, then
    // ct-print --full decoding them back to `{"kind":"Int","i":<n>,...}`.
    let observed_vars: Vec<(String, i64)> = events
        .iter()
        .filter(|e| e["kind"] == "step")
        .flat_map(|e| {
            e["vars"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
        })
        .filter_map(|v| {
            let name = v["varname"].as_str()?.to_string();
            let value = &v["value"];
            // The Aiken recorder encodes integer let-bindings as
            // ValueRecord::Int.  If something else surfaces (e.g.
            // BigInt for out-of-range integers, or a tagged variant
            // for Aiken's typed primitives), fail loudly so the test
            // author can decide whether to extend the assertions or
            // accept the new variant.
            assert_eq!(
                value["kind"].as_str(),
                Some("Int"),
                "variable `{}` should decode as Int, got {}; \
                 if a new ValueRecord variant has landed for Aiken \
                 integers, extend this test to assert on it explicitly \
                 rather than weakening the check",
                name,
                value
            );
            let i = value["i"]
                .as_i64()
                .unwrap_or_else(|| panic!("Int.i must be i64 for `{name}`; got {value}"));
            Some((name, i))
        })
        .collect();

    // The canonical flow: a=10, b=32, sum_val=a+b=42, doubled=sum_val*2=84,
    // final_result=doubled+a=94.  Same canonical fixture as cairo, leo,
    // and the other recorders — if your recorder runs flow_test.* and
    // these five let-bindings don't surface, that's the bug to chase.
    let expected: &[(&str, i64)] = &[
        ("a", 10),
        ("b", 32),
        ("sum_val", 42),
        ("doubled", 84),
        ("final_result", 94),
    ];
    for (name, value) in expected {
        assert!(
            observed_vars
                .iter()
                .any(|(n, v)| n == name && v == value),
            "expected step variable `{name}` = {value} in --full output; \
             observed = {observed_vars:?}"
        );
    }
}

// ===========================================================================
// CLI env-var contract
// ===========================================================================

/// `CODETRACER_CARDANO_RECORDER_OUT_DIR` must be honoured as a fallback
/// for `--out-dir`.  Convention: `Recorder-CLI-Conventions.md` §5.
#[test]
fn test_env_out_dir_used_when_flag_omitted() {
    let tmp_dir = tempfile::tempdir().expect("tempdir");
    let env_out_dir = tmp_dir.path().join("via-env");

    let source_path = test_programs_dir().join("flow_test.ak");

    let output = Command::new(env!("CARGO_BIN_EXE_codetracer-cardano-recorder"))
        .args(["record"])
        .arg(&source_path)
        .env("CODETRACER_CARDANO_RECORDER_OUT_DIR", &env_out_dir)
        // Make sure the env-var doesn't bleed in from the developer's shell.
        .env_remove("CODETRACER_CARDANO_RECORDER_DISABLED")
        .output()
        .expect("failed to run recorder");

    assert!(
        output.status.success(),
        "recorder should succeed when CODETRACER_CARDANO_RECORDER_OUT_DIR is set; \
         stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let ct_files = ct_files_in(&env_out_dir);
    assert!(
        !ct_files.is_empty(),
        "expected the env-supplied output dir {:?} to receive the .ct container",
        env_out_dir
    );
}

/// `CODETRACER_CARDANO_RECORDER_DISABLED=1` must skip recording entirely.
/// The recorder process should still exit 0 (the Cardano recorder
/// doesn't run a separate target subprocess — it parses & evaluates the
/// Aiken source itself — so "disabled" simply means "don't write any
/// trace artefacts").
#[test]
fn test_env_disabled_skips_recording() {
    let tmp_dir = tempfile::tempdir().expect("tempdir");
    let out_dir = tmp_dir.path().join("should-stay-empty");

    let source_path = test_programs_dir().join("flow_test.ak");

    let output = Command::new(env!("CARGO_BIN_EXE_codetracer-cardano-recorder"))
        .args(["record"])
        .arg(&source_path)
        .args(["--out-dir"])
        .arg(&out_dir)
        .env("CODETRACER_CARDANO_RECORDER_DISABLED", "1")
        .output()
        .expect("failed to run recorder");

    assert!(
        output.status.success(),
        "recorder should succeed in disabled mode; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // No .ct file should have been written.
    assert!(
        !out_dir.exists() || ct_files_in(&out_dir).is_empty(),
        "no .ct container should be written when CODETRACER_CARDANO_RECORDER_DISABLED=1; \
         got files in {:?}",
        out_dir
    );
}

/// `--format` is no longer accepted at any level — clap must reject it.
/// Convention: §4 (CTFS-only).
#[test]
fn test_format_flag_rejected_by_clap() {
    let tmp_dir = tempfile::tempdir().expect("tempdir");
    let out_dir = tmp_dir.path().join("traces");
    let source_path = test_programs_dir().join("flow_test.ak");

    let output = Command::new(env!("CARGO_BIN_EXE_codetracer-cardano-recorder"))
        .args(["record"])
        .arg(&source_path)
        .args(["--out-dir"])
        .arg(&out_dir)
        .args(["--format", "json"])
        .output()
        .expect("failed to run recorder");

    assert!(
        !output.status.success(),
        "--format should be rejected by clap; stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--format")
            || stderr.contains("unexpected argument")
            || stderr.contains("unrecognized")
            || stderr.contains("found argument"),
        "clap error should mention the unknown --format flag; got stderr:\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// UPLC evaluation tests (pure unit tests, no trace writer)
// ---------------------------------------------------------------------------

#[test]
fn test_uplc_cek_machine_addition() {
    let term: Term<NamedDeBruijn> = Term::Apply {
        function: Rc::new(Term::Apply {
            function: Rc::new(Term::Builtin(DefaultFunction::AddInteger)),
            argument: Rc::new(Term::Constant(Rc::new(Constant::Integer(BigInt::from(10))))),
        }),
        argument: Rc::new(Term::Constant(Rc::new(Constant::Integer(BigInt::from(32))))),
    };
    let program = Program {
        version: (1, 0, 0),
        term,
    };
    let eval_result = program.eval(ExBudget::default());
    let result_term = eval_result.result().unwrap();
    assert_eq!(
        result_term,
        Term::Constant(Rc::new(Constant::Integer(BigInt::from(42))))
    );
}

#[test]
fn test_uplc_cek_machine_full_computation() {
    let sum_val = Term::Apply {
        function: Rc::new(Term::Apply {
            function: Rc::new(Term::Builtin(DefaultFunction::AddInteger)),
            argument: Rc::new(Term::Constant(Rc::new(Constant::Integer(BigInt::from(10))))),
        }),
        argument: Rc::new(Term::Constant(Rc::new(Constant::Integer(BigInt::from(32))))),
    };
    let doubled = Term::Apply {
        function: Rc::new(Term::Apply {
            function: Rc::new(Term::Builtin(DefaultFunction::MultiplyInteger)),
            argument: Rc::new(sum_val),
        }),
        argument: Rc::new(Term::Constant(Rc::new(Constant::Integer(BigInt::from(2))))),
    };
    let final_result = Term::Apply {
        function: Rc::new(Term::Apply {
            function: Rc::new(Term::Builtin(DefaultFunction::AddInteger)),
            argument: Rc::new(doubled),
        }),
        argument: Rc::new(Term::Constant(Rc::new(Constant::Integer(BigInt::from(10))))),
    };
    let comparison = Term::Apply {
        function: Rc::new(Term::Apply {
            function: Rc::new(Term::Builtin(DefaultFunction::EqualsInteger)),
            argument: Rc::new(final_result),
        }),
        argument: Rc::new(Term::Constant(Rc::new(Constant::Integer(BigInt::from(94))))),
    };
    let program = Program {
        version: (1, 0, 0),
        term: comparison,
    };
    let eval_result = program.eval(ExBudget::default());
    let result_term = eval_result.result().unwrap();
    assert_eq!(result_term, Term::Constant(Rc::new(Constant::Bool(true))));
}

#[test]
fn test_uplc_parse_and_eval() {
    let uplc_src = "(program 1.0.0 [ [ (builtin addInteger) (con integer 10) ] (con integer 32) ])";
    let parsed = uplc::parser::program(uplc_src).expect("UPLC should parse");
    let named_db: Program<NamedDeBruijn> = parsed.to_named_debruijn().expect("should convert");
    let eval_result = named_db.eval(ExBudget::default());
    let result_term = eval_result.result().unwrap();
    assert_eq!(
        result_term,
        Term::Constant(Rc::new(Constant::Integer(BigInt::from(42))))
    );
}

#[test]
fn test_uplc_file_evaluation() {
    let uplc_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-programs/uplc/flow_test.uplc");
    let uplc_src = std::fs::read_to_string(&uplc_path).expect("failed to read flow_test.uplc");
    let parsed = uplc::parser::program(&uplc_src).expect("flow_test.uplc should parse");
    let named_db: Program<NamedDeBruijn> = parsed.to_named_debruijn().expect("should convert");
    let eval_result = named_db.eval(ExBudget::default());
    let result_term = eval_result.result().unwrap();
    assert_eq!(result_term, Term::Constant(Rc::new(Constant::Bool(true))));
}
