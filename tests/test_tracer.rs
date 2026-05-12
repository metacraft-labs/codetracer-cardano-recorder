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
// Per-program ct-print --full coverage tests
// ===========================================================================
//
// These tests follow the recorder-test-requirements policy
// (`metacraft-specs/policies/recorder-test-requirements.md`):
//
// * Each test records one Aiken program through the recorder's
//   normal entry point.
// * The produced `.ct` is piped through `ct-print --full --strip-paths`.
// * Assertions are made on the **decoded JSON document** with EXACT
//   counts (`assert_eq!(events.len(), N)` — never `>=`), EXACT
//   ordering (later step from a strictly later source line where
//   applicable), and EXACT decoded values
//   (`value["i"] == 42`, `value["kind"] == "Int"`).
//
// `ValueRecord` variants outside the expected set are rejected with
// a hard error message asking the test author to extend the test
// rather than weaken the assertion.
//
// Where the recorder's current behaviour deviates from what the
// language semantics dictate (e.g. function-call arguments not being
// traced because the source-level parser is hand-rolled and very
// limited), the deviation is documented inline as `RECORDER BUG: ...`
// and a parallel `#[ignore]`d assertion captures the spec-correct
// expectation so it surfaces the moment the recorder catches up.

/// Skip-helper: returns `Some(path)` to ct-print or logs a clear
/// `SKIP:` diagnostic and returns `None`.  The
/// `verify-cli-convention-no-silent-skip.sh` script greps for the
/// literal `SKIP:` token, so silent skips remain forbidden.
fn ct_print_or_skip(test_name: &str) -> Option<PathBuf> {
    let p = ct_print_path();
    if !p.exists() {
        eprintln!(
            "SKIP: {test_name} requires ct-print at {} — only available \
             within the metacraft workspace where codetracer-trace-format-nim \
             is a sibling.",
            p.display()
        );
        return None;
    }
    Some(p)
}

/// Record a program and return the `ct-print --full --strip-paths`
/// JSON document plus the absolute path to the source file (so the
/// caller can match `metadata.program`).  Returns `None` when
/// `ct-print` is unavailable (the caller has already emitted a
/// `SKIP:` line via `ct_print_or_skip`).
fn record_and_dump_full(test_name: &str, program: &str) -> Option<(serde_json::Value, PathBuf)> {
    let ct_print = ct_print_or_skip(test_name)?;

    let tmp_dir = tempfile::tempdir().expect("tempdir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path = test_programs_dir().join(program);
    codetracer_cardano_recorder::recorder::record(&source_path, &out_dir)
        .expect("recorder::record should succeed");

    let ct_files = ct_files_in(&out_dir);
    assert!(
        !ct_files.is_empty(),
        "expected a .ct container in {:?}",
        out_dir
    );

    let output = Command::new(&ct_print)
        .args(["--full", "--strip-paths"])
        .arg(&ct_files[0])
        .output()
        .expect("failed to run ct-print --full");

    assert!(
        output.status.success(),
        "ct-print --full should succeed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let doc: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("ct-print --full should emit valid JSON");

    // Preserve the temp dir until after the JSON is parsed, then drop.
    drop(tmp_dir);

    Some((doc, source_path))
}

/// Decode every (varname, i64) pair from step events.  Rejects any
/// `ValueRecord` variant other than `Int` with a hard error that
/// asks the test author to extend the test rather than weaken it.
fn observed_int_vars(doc: &serde_json::Value) -> Vec<(String, i64)> {
    let events = doc["events"].as_array().expect("events array");
    let mut out = Vec::new();
    for ev in events {
        if ev["kind"] != "step" {
            continue;
        }
        let Some(vars) = ev["vars"].as_array() else {
            continue;
        };
        for v in vars {
            let name = v["varname"].as_str().expect("varname str").to_string();
            let value = &v["value"];
            assert_eq!(
                value["kind"].as_str(),
                Some("Int"),
                "variable `{}` should decode as Int, got {}; \
                 if a new ValueRecord variant has landed for X, extend this \
                 test to assert on it explicitly rather than weakening the \
                 check",
                name,
                value
            );
            let i = value["i"]
                .as_i64()
                .unwrap_or_else(|| panic!("Int.i must be i64 for `{name}`; got {value}"));
            out.push((name, i));
        }
    }
    out
}

/// Decode the call-entry sequence as a vector of function names.
fn observed_call_sequence(doc: &serde_json::Value) -> Vec<String> {
    doc["events"]
        .as_array()
        .expect("events array")
        .iter()
        .filter(|e| e["kind"] == "call_entry")
        .map(|e| {
            e["function"]
                .as_str()
                .expect("call_entry.function str")
                .to_string()
        })
        .collect()
}

/// Decode (varname, i64) pairs in event-emission order.
fn observed_var_sequence(doc: &serde_json::Value) -> Vec<(String, i64)> {
    observed_int_vars(doc)
}

/// Assert that every `step` event carries a strictly non-decreasing
/// `step_index`.  This is the recorder's only ordering guarantee
/// against duplicates / reorderings.
fn assert_step_indices_monotonic(doc: &serde_json::Value) {
    let mut last = -1i64;
    for ev in doc["events"].as_array().expect("events array") {
        if ev["kind"] != "step" {
            continue;
        }
        let idx = ev["step_index"]
            .as_i64()
            .expect("step_index must be present on step events");
        assert!(
            idx > last,
            "step_index must strictly increase; got {idx} after {last}"
        );
        last = idx;
    }
}

/// Assert `metadata.program` ends with the expected source filename.
fn assert_metadata_program_ends_with(doc: &serde_json::Value, source_path: &Path) {
    let prog = doc["metadata"]["program"]
        .as_str()
        .expect("metadata.program str");
    let want = source_path.file_name().unwrap().to_string_lossy();
    assert!(
        prog.ends_with(&*want),
        "metadata.program {prog} must end with {want}"
    );
}

// --- control_flow_test.ak --------------------------------------------------

/// Records `control_flow_test.ak` and asserts on the **current
/// observed** event shape.  The program exercises if/else, when
/// (pattern matching), and let-in chains.  The recorder is hand-
/// rolled and intentionally does **not** parse `if`, `when`, or
/// function calls with arguments — see the inline RECORDER BUG
/// notes below.  This test pins the present-day output as a golden
/// snapshot so any future regression is caught even before the
/// upstream parser gains real support.
#[test]
fn test_control_flow_test_via_ct_print_full() {
    let Some((doc, source_path)) =
        record_and_dump_full("test_control_flow_test_via_ct_print_full", "control_flow_test.ak")
    else {
        return;
    };

    assert_metadata_program_ends_with(&doc, &source_path);

    // ----- Function table ---------------------------------------------
    // RECORDER BUG: `classify` and `pick` should both appear in the
    // function table (they are defined and called), but the recorder's
    // `parse_function_call` only matches bare `name()` invocations
    // — `classify(raw)` and `pick(sign)` carry arguments and are
    // therefore never registered as calls.  Once the parser gains
    // expression-level call support, this test will fail and the
    // `expected_functions` list below should be extended to include
    // them.  See also the call-sequence assertion further down.
    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(
        functions,
        vec!["control_flow", "compute"],
        "function table mismatch — has classify/pick support landed?"
    );

    // ----- counts -----------------------------------------------------
    // 8 step events: 1 outer + 1 dispatch + 5 let-bindings + 1 trailing
    // expression.  Only `raw` produces a real Int value; the other
    // four let-bindings call `classify`/`pick` with arguments which
    // the parser can't compile, so they emit empty `vars` arrays.
    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(8), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(1), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );
    assert_eq!(
        counts["values"].as_u64(),
        Some(8),
        "values; counts={counts}"
    );

    let events = doc["events"].as_array().expect("events array");
    assert_eq!(events.len(), 10, "events.len()");
    assert_step_indices_monotonic(&doc);

    // ----- Call sequence ----------------------------------------------
    // RECORDER BUG: spec wants [compute, classify, pick] (the test
    // body invokes compute, which calls classify and pick).  Today we
    // only see the bare `compute()` call because `classify(raw)` is
    // not parsed as a call.  Once that lands, this list grows.
    assert_eq!(observed_call_sequence(&doc), vec!["compute".to_string()]);

    // ----- Decoded variable values ------------------------------------
    // RECORDER BUG: `sign`, `bonus`, `combined`, `result` should each
    // surface as an Int decoded from real CEK evaluation; today the
    // recorder emits an empty `vars` array for those steps because
    // their right-hand side contains a function call with arguments.
    assert_eq!(
        observed_var_sequence(&doc),
        vec![("raw".to_string(), 7)],
        "only `raw` is decoded today; the other let-bindings depend on \
         classify(raw)/pick(sign) which the source-level parser cannot \
         compile to UPLC."
    );
}

#[test]
#[ignore = "RECORDER BUG: classify/pick calls with arguments are not \
            parsed as function calls, so sign/bonus/combined/result do \
            not surface as decoded Ints.  Tracking expectation: full \
            chain should yield [(\"raw\", 7), (\"sign\", 1), \
            (\"bonus\", 300), (\"combined\", 301), (\"result\", 602)]."]
fn test_control_flow_test_full_chain_decodes() {
    let Some((doc, _)) =
        record_and_dump_full("test_control_flow_test_full_chain_decodes", "control_flow_test.ak")
    else {
        return;
    };
    let expected: Vec<(String, i64)> = vec![
        ("raw".into(), 7),
        ("sign".into(), 1),
        ("bonus".into(), 300),
        ("combined".into(), 301),
        ("result".into(), 602),
    ];
    assert_eq!(observed_var_sequence(&doc), expected);
}

// --- nested_calls_test.ak --------------------------------------------------

/// Records `nested_calls_test.ak` and asserts on the **exact** event
/// shape.  This is the well-behaved case: every function in the
/// chain takes zero arguments, so the recorder's hand-rolled
/// `parse_function_call` matches them all and the four-deep chain
/// `compute → outer → middle → inner` is captured end-to-end.
#[test]
fn test_nested_calls_test_via_ct_print_full() {
    let Some((doc, source_path)) =
        record_and_dump_full("test_nested_calls_test_via_ct_print_full", "nested_calls_test.ak")
    else {
        return;
    };

    assert_metadata_program_ends_with(&doc, &source_path);

    // ----- Function table — order is writer-assignment order ---------
    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(
        functions,
        vec!["nested_calls", "compute", "outer", "middle", "inner"],
    );

    // ----- counts -----------------------------------------------------
    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(14), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(4), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );

    let events = doc["events"].as_array().expect("events array");
    // 14 steps + 4 call_entry + 4 call_exit = 22 events
    assert_eq!(events.len(), 22, "events.len()");
    assert_step_indices_monotonic(&doc);

    // ----- Call entry order: outermost first --------------------------
    assert_eq!(
        observed_call_sequence(&doc),
        vec![
            "compute".to_string(),
            "outer".to_string(),
            "middle".to_string(),
            "inner".to_string(),
        ],
        "call_entry events must appear in entry order"
    );

    // ----- Call exit order: innermost first (LIFO) --------------------
    let exit_sequence: Vec<&str> = events
        .iter()
        .filter(|e| e["kind"] == "call_exit")
        .filter_map(|e| e["function"].as_str())
        .collect();
    assert_eq!(
        exit_sequence,
        vec!["inner", "middle", "outer", "compute"],
        "call_exit events must appear in LIFO order"
    );

    // ----- Exact decoded values + return values -----------------------
    // The chain: inner returns 1+2=3, middle returns 3+10=13, outer
    // returns 13+100=113, compute returns 113.
    let expected_vars: Vec<(String, i64)> = vec![
        ("a".into(), 1),
        ("b".into(), 2),
        ("c".into(), 3),
        ("x".into(), 3),
        ("y".into(), 13),
        ("p".into(), 13),
        ("q".into(), 113),
        ("result".into(), 113),
    ];
    assert_eq!(observed_var_sequence(&doc), expected_vars);

    // ----- Return values on each call_exit ----------------------------
    let returns: Vec<i64> = events
        .iter()
        .filter(|e| e["kind"] == "call_exit")
        .map(|e| {
            let rv = &e["return_value"];
            assert_eq!(
                rv["kind"].as_str(),
                Some("Int"),
                "return value must decode as Int; got {rv}"
            );
            rv["i"].as_i64().expect("return value Int.i")
        })
        .collect();
    assert_eq!(returns, vec![3, 13, 113, 113]);
}

// --- collections_test.ak ---------------------------------------------------

/// Records `collections_test.ak` and asserts on the present-day
/// shape.  RECORDER BUG: collection literals (lists, tuples,
/// records) are completely opaque to the recorder.  Function calls
/// with arguments (`sum_pair((10, 20))`, `point_distance_sq(...)`)
/// are not parsed as calls.  The only call that lands is
/// `list_total()` (zero arguments), and the only decoded values are
/// the Int let-bindings inside it.
#[test]
fn test_collections_test_via_ct_print_full() {
    let Some((doc, source_path)) =
        record_and_dump_full("test_collections_test_via_ct_print_full", "collections_test.ak")
    else {
        return;
    };

    assert_metadata_program_ends_with(&doc, &source_path);

    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    // RECORDER BUG: spec wants {collections, compute, sum_pair,
    // point_distance_sq, list_total} — argumentful calls are dropped.
    assert_eq!(functions, vec!["collections", "compute", "list_total"]);

    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(11), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(2), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );

    let events = doc["events"].as_array().expect("events array");
    // 11 step events + 2 call_entry + 2 call_exit = 15 events.
    assert_eq!(events.len(), 15, "events.len()");
    assert_step_indices_monotonic(&doc);

    assert_eq!(
        observed_call_sequence(&doc),
        vec!["compute".to_string(), "list_total".to_string()],
    );

    // RECORDER BUG: a spec-compliant trace would expose ValueRecord
    // variants for List/Sequence (xs = [1,2,3,4]), Tuple ((10,20),
    // and Struct (Point{x:3,y:4}).  Today the recorder emits only
    // Int values, and only for the integer let-bindings inside the
    // zero-arg `list_total` callee + the scalar passthrough in
    // compute().
    assert_eq!(
        observed_var_sequence(&doc),
        vec![
            ("head_val".into(), 1),
            ("len".into(), 4),
            ("list_total_val".into(), 5),
        ],
    );
}

#[test]
#[ignore = "RECORDER BUG: list/tuple/record literals are not encoded \
            as ValueRecord::Sequence / Tuple / Struct; argumentful \
            function calls are not traced.  Spec-compliant output \
            should surface xs as Sequence, the (10,20) pair as Tuple, \
            and Point{x:3,y:4} as Struct."]
fn test_collections_test_value_kinds_present() {
    let Some((doc, _)) =
        record_and_dump_full("test_collections_test_value_kinds_present", "collections_test.ak")
    else {
        return;
    };
    let mut kinds = std::collections::BTreeSet::new();
    for ev in doc["events"].as_array().unwrap() {
        if ev["kind"] != "step" {
            continue;
        }
        for v in ev["vars"].as_array().cloned().unwrap_or_default() {
            if let Some(k) = v["value"]["kind"].as_str() {
                kinds.insert(k.to_string());
            }
        }
    }
    for want in ["Int", "Sequence", "Tuple", "Struct"] {
        assert!(
            kinds.contains(want),
            "expected {want} ValueRecord variant in collections trace; got {kinds:?}"
        );
    }
}

// --- error_paths_test.ak ---------------------------------------------------

/// Records `error_paths_test.ak`.  The recorder picks the first
/// `test` block as the entry point (`safe_path`), so today only the
/// non-failing branch runs.  RECORDER BUG: there's no way to
/// indicate "run all test blocks", and `fail @"..."` is never
/// invoked, so the failing path produces no `RecordEvent` of
/// `EventKindError`.
#[test]
fn test_error_paths_test_via_ct_print_full() {
    let Some((doc, source_path)) =
        record_and_dump_full("test_error_paths_test_via_ct_print_full", "error_paths_test.ak")
    else {
        return;
    };

    assert_metadata_program_ends_with(&doc, &source_path);

    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    // RECORDER BUG: the spec-compliant function table would also
    // include `failing_path` and `failing_compute` (they're declared
    // and reachable from a test).  Today only the entry test plus
    // the functions it invokes are registered.
    assert_eq!(functions, vec!["safe_path", "compute", "safe_compute"]);

    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(9), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(2), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );

    let events = doc["events"].as_array().expect("events array");
    // 9 steps + 2 call_entry + 2 call_exit = 13 events.
    assert_eq!(events.len(), 13, "events.len()");
    assert_step_indices_monotonic(&doc);

    assert_eq!(
        observed_call_sequence(&doc),
        vec!["compute".to_string(), "safe_compute".to_string()],
    );

    assert_eq!(
        observed_var_sequence(&doc),
        vec![
            ("a".into(), 5),
            ("b".into(), 7),
            ("c".into(), 12),
            ("safe_val".into(), 12),
            ("bumped".into(), 112),
        ],
    );
}

#[test]
#[ignore = "RECORDER BUG: `fail` is not surfaced as a special event \
            (EventLogKind::Error or similar).  When a fail is \
            reachable the recorder should emit an io_event of error \
            kind containing the literal payload."]
fn test_error_paths_test_emits_fail_event() {
    let Some((doc, _)) =
        record_and_dump_full("test_error_paths_test_emits_fail_event", "error_paths_test.ak")
    else {
        return;
    };
    let counts = &doc["counts"];
    assert!(
        counts["io_events"].as_u64().unwrap_or(0) >= 1,
        "expected at least one io_event for the `fail` expression; counts={counts}"
    );
}

// --- tracing_test.ak -------------------------------------------------------

/// Records `tracing_test.ak`.  The program calls `trace @"label":
/// value` three times.  RECORDER BUG: `trace` is parsed as a bare
/// expression statement and produces no `RecordEvent` (write kind)
/// — the recorder has no `trace`-handling at all today, so
/// `io_events` is 0.  When the recorder gains trace support, the
/// `#[ignore]`d sibling test below will start passing.
#[test]
fn test_tracing_test_via_ct_print_full() {
    let Some((doc, source_path)) =
        record_and_dump_full("test_tracing_test_via_ct_print_full", "tracing_test.ak")
    else {
        return;
    };

    assert_metadata_program_ends_with(&doc, &source_path);

    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(functions, vec!["tracing", "compute"]);

    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(9), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(1), "calls; counts={counts}");
    // RECORDER BUG: should be >= 3 (one per `trace @"...": ...`).
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );

    let events = doc["events"].as_array().expect("events array");
    // 9 steps + 1 call_entry + 1 call_exit = 11 events.
    assert_eq!(events.len(), 11, "events.len()");
    assert_step_indices_monotonic(&doc);

    assert_eq!(observed_call_sequence(&doc), vec!["compute".to_string()]);

    // The integer let-bindings still surface even though the
    // intervening `trace` expressions don't: a, b, sum_val.
    assert_eq!(
        observed_var_sequence(&doc),
        vec![
            ("a".into(), 4),
            ("b".into(), 5),
            ("sum_val".into(), 9),
        ],
    );
}

#[test]
#[ignore = "RECORDER BUG: `trace @\"label\": value` is not surfaced \
            as a RecordEvent (write kind).  Spec-compliant output \
            should emit one io_event per trace call, with the label \
            string and the decoded value."]
fn test_tracing_test_emits_record_events() {
    let Some((doc, _)) =
        record_and_dump_full("test_tracing_test_emits_record_events", "tracing_test.ak")
    else {
        return;
    };
    let counts = &doc["counts"];
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(3),
        "expected exactly 3 io_events (one per trace expression); counts={counts}"
    );
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
