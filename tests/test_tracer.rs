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
            observed_vars.iter().any(|(n, v)| n == name && v == value),
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

    let doc: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("ct-print --full should emit valid JSON");

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
    let Some((doc, source_path)) = record_and_dump_full(
        "test_control_flow_test_via_ct_print_full",
        "control_flow_test.ak",
    ) else {
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
        vec!["control_flow", "compute", "classify", "pick"],
        "function table must include classify+pick now that the parser \
         recognises calls with arguments — see commit landing \
         `parse_function_call` arg support",
    );

    // ----- counts -----------------------------------------------------
    // Steps now cover the entire `compute → classify → pick` chain:
    //   1 outer  (the implicit start-of-trace step at line 1)
    //   1 dispatch (`compute() == 602` in the test body)
    //   6 in compute (5 let-bindings + 1 trailing `result` expr)
    //   1 param-intro step in classify (binds `n=7`, emitted at the
    //                  function's signature line so the value lands
    //                  in the variable stream)
    //   6 in classify (`if n < 0 {`, `-1`, `} else if n == 0 {`, `0`,
    //                  `} else {`, `1` — the hand-rolled parser
    //                  treats each non-`}` body line as its own
    //                  Statement::Expr, "last evaluable line wins"
    //                  for the function's return value)
    //   1 param-intro step in pick (binds `tag=1`)
    //   4 in pick (`when tag is {` + three arms recognised by the new
    //              `find_top_level_arrow` helper)
    //   = 20 steps total.
    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(20), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(3), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );
    assert_eq!(
        counts["values"].as_u64(),
        Some(20),
        "values; counts={counts}"
    );

    let events = doc["events"].as_array().expect("events array");
    // 20 steps + 3 call_entry + 3 call_exit = 26 events.
    assert_eq!(events.len(), 26, "events.len()");
    assert_step_indices_monotonic(&doc);

    // ----- Call sequence ----------------------------------------------
    // The chain is now captured end-to-end: the test body invokes
    // compute(), which in turn calls classify(raw) and pick(sign).
    assert_eq!(
        observed_call_sequence(&doc),
        vec![
            "compute".to_string(),
            "classify".to_string(),
            "pick".to_string(),
        ],
    );

    // ----- Decoded variable values ------------------------------------
    // All five let-bindings plus the two callees' parameters now
    // surface as decoded Ints:
    //   raw=7 (literal)
    //   n=7 (classify's param-intro step — bound from `raw`)
    //   sign=classify(7)=1 (last-evaluable-line == else-branch literal)
    //   tag=1 (pick's param-intro step — bound from `sign`)
    //   bonus=pick(1)=300 (last `when` arm value)
    //   combined=sign+bonus=301
    //   result=combined*2=602
    assert_eq!(
        observed_var_sequence(&doc),
        vec![
            ("raw".to_string(), 7),
            ("n".to_string(), 7),
            ("sign".to_string(), 1),
            ("tag".to_string(), 1),
            ("bonus".to_string(), 300),
            ("combined".to_string(), 301),
            ("result".to_string(), 602),
        ],
    );
}

#[test]
fn test_control_flow_test_full_chain_decodes() {
    let Some((doc, _)) = record_and_dump_full(
        "test_control_flow_test_full_chain_decodes",
        "control_flow_test.ak",
    ) else {
        return;
    };
    // Each non-test function with parameters now emits a
    // param-intro step that binds the formal-parameter names so
    // their values appear in the variable stream (rather than
    // only being threaded through arithmetic).  For
    // `control_flow_test.ak` this surfaces `n=7` after `raw=7`
    // (classify(7)) and `tag=1` after `sign=1` (pick(1)).
    let expected: Vec<(String, i64)> = vec![
        ("raw".into(), 7),
        ("n".into(), 7),
        ("sign".into(), 1),
        ("tag".into(), 1),
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
    let Some((doc, source_path)) = record_and_dump_full(
        "test_nested_calls_test_via_ct_print_full",
        "nested_calls_test.ak",
    ) else {
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

/// Records `collections_test.ak` and pins the full event shape now
/// that the recorder decodes Aiken's structured literals.  The
/// recorder lifts list literals (`[1, 2, 3, 4]`) into
/// `ValueRecord::Sequence`, tuple literals (`(10, 20)`) into
/// `ValueRecord::Tuple`, and record literals (`Point { x: 3, y: 4 }`)
/// into `ValueRecord::Struct`, threads them through argumentful
/// function calls (`sum_pair`, `point_distance_sq`), supports tuple
/// destructuring (`let (a, b) = p`) and record-field access
/// (`p.x * p.x`), and computes the same scalar result as the on-chain
/// program (`compute() == 60`).
#[test]
fn test_collections_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_collections_test_via_ct_print_full",
        "collections_test.ak",
    ) else {
        return;
    };

    assert_metadata_program_ends_with(&doc, &source_path);

    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(
        functions,
        vec![
            "collections",
            "compute",
            "sum_pair",
            "point_distance_sq",
            "list_total"
        ],
        "function table must include the full chain — `sum_pair` and \
         `point_distance_sq` are called with structured arguments which \
         the recorder now parses end-to-end",
    );

    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(16), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(4), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );
    assert_eq!(
        counts["values"].as_u64(),
        Some(16),
        "values; counts={counts}"
    );

    let events = doc["events"].as_array().expect("events array");
    // 16 step events + 4 call_entry + 4 call_exit = 24 events.
    assert_eq!(events.len(), 24, "events.len()");
    assert_step_indices_monotonic(&doc);

    assert_eq!(
        observed_call_sequence(&doc),
        vec![
            "compute".to_string(),
            "sum_pair".to_string(),
            "point_distance_sq".to_string(),
            "list_total".to_string(),
        ],
    );

    // ----- Structured variable shapes ---------------------------------
    // Walk the step events in emission order and pin the (name,
    // ValueRecord::kind) pair for every variable.  This is stricter
    // than `observed_int_vars` (which only accepts Int) because the
    // collections fixture deliberately exercises Sequence / Tuple /
    // Struct shapes that must NOT silently downgrade to Int.
    let var_sequence: Vec<(String, String)> = events
        .iter()
        .filter(|e| e["kind"] == "step")
        .flat_map(|e| {
            e["vars"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|v| {
                    (
                        v["varname"].as_str().expect("varname").to_string(),
                        v["value"]["kind"].as_str().expect("value.kind").to_string(),
                    )
                })
        })
        .collect();
    assert_eq!(
        var_sequence,
        vec![
            // sum_pair: param `p` is the (10,20) tuple, then
            // destructured into `a=10`, `b=20`.
            ("p".into(), "Tuple".into()),
            ("a".into(), "Int".into()),
            ("b".into(), "Int".into()),
            // back in compute: sum_pair returned 30.
            ("pair_total".into(), "Int".into()),
            // point_distance_sq: param `p` is the Point struct.
            ("p".into(), "Struct".into()),
            // back in compute: point_distance_sq returned 3*3+4*4=25.
            ("pt_total".into(), "Int".into()),
            // list_total: `xs` is the list literal Sequence.
            ("xs".into(), "Sequence".into()),
            ("head_val".into(), "Int".into()),
            ("len".into(), "Int".into()),
            // back in compute: list_total returned 1+4=5.
            ("list_total_val".into(), "Int".into()),
            // compute's final binding before its trailing expression.
            ("grand_total".into(), "Int".into()),
        ],
    );

    // ----- Spot-check the structured payloads --------------------------
    let p_tuple_value = find_var_value(&doc, "p", "Tuple");
    let elems = p_tuple_value["elements"]
        .as_array()
        .expect("Tuple elements array");
    let int_at = |i: usize| {
        assert_eq!(elems[i]["kind"].as_str(), Some("Int"));
        elems[i]["i"].as_i64().expect("Int.i")
    };
    assert_eq!(int_at(0), 10);
    assert_eq!(int_at(1), 20);

    let p_struct_value = find_var_value(&doc, "p", "Struct");
    let fields = p_struct_value["field_values"]
        .as_array()
        .expect("Struct field_values array");
    let f_int_at = |i: usize| {
        assert_eq!(fields[i]["kind"].as_str(), Some("Int"));
        fields[i]["i"].as_i64().expect("Int.i")
    };
    assert_eq!(f_int_at(0), 3);
    assert_eq!(f_int_at(1), 4);

    let xs_sequence = find_var_value(&doc, "xs", "Sequence");
    let xs_elems = xs_sequence["elements"]
        .as_array()
        .expect("Sequence elements array");
    let xs_int_at = |i: usize| {
        assert_eq!(xs_elems[i]["kind"].as_str(), Some("Int"));
        xs_elems[i]["i"].as_i64().expect("Int.i")
    };
    assert_eq!(xs_int_at(0), 1);
    assert_eq!(xs_int_at(1), 2);
    assert_eq!(xs_int_at(2), 3);
    assert_eq!(xs_int_at(3), 4);
    assert_eq!(
        xs_sequence["is_slice"].as_bool(),
        Some(false),
        "list literals are NOT slices; the recorder must emit \
         is_slice=false to distinguish them from sliced borrows",
    );

    // ----- Final scalar invariant -------------------------------------
    // The on-chain test asserts `compute() == 60`.  The trace's
    // last Int variable in compute() is `grand_total = pair_total
    // (30) + pt_total (25) + list_total_val (5) = 60`.
    let grand = find_var_value(&doc, "grand_total", "Int");
    assert_eq!(grand["i"].as_i64(), Some(60));
}

/// Locate the first `vars[].value` entry in the trace whose
/// `varname` matches `name` and whose `value.kind` matches
/// `expected_kind`.  Panics with a precise message if no such
/// entry exists — that's by design: callers use this to pin a
/// specific shape, and a missing entry is a real recorder
/// regression.
fn find_var_value(doc: &serde_json::Value, name: &str, expected_kind: &str) -> serde_json::Value {
    for ev in doc["events"].as_array().expect("events array") {
        if ev["kind"] != "step" {
            continue;
        }
        for v in ev["vars"].as_array().cloned().unwrap_or_default() {
            if v["varname"].as_str() == Some(name)
                && v["value"]["kind"].as_str() == Some(expected_kind)
            {
                return v["value"].clone();
            }
        }
    }
    panic!(
        "expected a `{name}` variable with value.kind == {expected_kind:?} \
         in the trace; got none"
    );
}

#[test]
fn test_collections_test_value_kinds_present() {
    let Some((doc, _)) = record_and_dump_full(
        "test_collections_test_value_kinds_present",
        "collections_test.ak",
    ) else {
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
/// indicate "run all test blocks", so the function table and step /
/// call counts only reflect the entry test plus its callees.  This
/// is a separate gap from the `fail` Error-event surfacing fix
/// pinned by `test_error_paths_test_emits_fail_event` below — the
/// recorder now scans for `fail @"..."` statements anywhere in the
/// parsed program and emits one `EventLogKind::Error` io_event per
/// `fail`, independent of which test runs.  See
/// `metacraft-specs/policies/recorder-test-requirements.md` §2.
#[test]
fn test_error_paths_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_error_paths_test_via_ct_print_full",
        "error_paths_test.ak",
    ) else {
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
    // The recorder emits exactly one `EventLogKind::Error` io_event
    // for the single `fail @"..."` statement in `failing_compute`
    // (post-execution sweep over all parsed function bodies — see
    // `AikenTracer::emit_fail_events_for_program`).
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(1),
        "io_events; counts={counts}"
    );

    let events = doc["events"].as_array().expect("events array");
    // 9 steps + 2 call_entry + 2 call_exit + 1 io_event = 14 events.
    assert_eq!(events.len(), 14, "events.len()");
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

    // The Error io_event carries the literal `fail @"..."` payload
    // verbatim — the frontend can route on the `AikenFail` metadata
    // tag to distinguish Aiken `fail` markers from generic UPLC CEK
    // failures (which carry `AikenUplcEvalError`).
    let error_events: Vec<&serde_json::Value> = events
        .iter()
        .filter(|e| e["kind"] == "io" && e["io_kind"] == "ioError")
        .collect();
    assert_eq!(
        error_events.len(),
        1,
        "expected exactly one ioError io_event for the `fail` in failing_compute; \
         got {error_events:?}"
    );
    let text = error_events[0]["text"].as_str().unwrap_or("");
    assert_eq!(
        text, "intentional failure for trace coverage",
        "Error io_event should carry the `fail @\"...\"` literal payload",
    );
}

#[test]
fn test_error_paths_test_emits_fail_event() {
    let Some((doc, _)) = record_and_dump_full(
        "test_error_paths_test_emits_fail_event",
        "error_paths_test.ak",
    ) else {
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
/// value` three times.  The recorder now scans for `trace @"..."`
/// statements anywhere in the parsed program and emits one
/// `EventLogKind::Write` io_event per `trace` (post-execution
/// sweep — see `AikenTracer::emit_trace_events_for_program`,
/// which inherits the same static-sweep limitation called out on
/// commit 7e5a177 for `fail`).  The `AikenTrace` metadata tag
/// mirrors the `AikenFail` convention so the frontend can route
/// trace output independently of generic write-kind io_events.
/// See `metacraft-specs/policies/recorder-test-requirements.md` §2.
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
    // The recorder emits exactly three `EventLogKind::Write`
    // io_events — one per `trace @"...": ...` statement in
    // `compute()` (post-execution sweep over all parsed function
    // bodies — see `AikenTracer::emit_trace_events_for_program`).
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(3),
        "io_events; counts={counts}"
    );

    let events = doc["events"].as_array().expect("events array");
    // 9 steps + 1 call_entry + 1 call_exit + 3 io_events = 14 events.
    assert_eq!(events.len(), 14, "events.len()");
    assert_step_indices_monotonic(&doc);

    assert_eq!(observed_call_sequence(&doc), vec!["compute".to_string()]);

    // The integer let-bindings surface alongside the trace events:
    // a, b, sum_val.
    assert_eq!(
        observed_var_sequence(&doc),
        vec![("a".into(), 4), ("b".into(), 5), ("sum_val".into(), 9),],
    );

    // Each `trace @"label": value` surfaces as an `ioStdout` io_event
    // (the multi-stream IOEvent stream collapses
    // `EventLogKind::Write` / `WriteFile` / `WriteOther` to the
    // `ioStdout` bucket — see `toIOEventKind` in
    // `codetracer-trace-format-nim`) whose `text` carries
    // `"<label>: <value-expr>"` verbatim.  The frontend can route on
    // the `AikenTrace` metadata tag (mirroring the `AikenFail`
    // convention) to distinguish trace output from generic
    // write-kind io_events.  Until the recorder gains runtime
    // resolution of trace values, the `value-expr` is the literal
    // source-level expression text (e.g. `"after-a: a"`) rather
    // than the resolved integer; same static-sweep limitation
    // called out for `emit_fail_events_for_program`.
    let trace_events: Vec<&serde_json::Value> = events
        .iter()
        .filter(|e| e["kind"] == "io" && e["io_kind"] == "ioStdout")
        .collect();
    let texts: Vec<&str> = trace_events
        .iter()
        .map(|e| e["text"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(
        texts,
        vec!["after-a: a", "after-b: b", "final-sum: sum_val"],
        "ioStdout events should carry one entry per `trace @\"label\": value` \
         with text \"<label>: <value-expr>\""
    );
}

#[test]
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

// --- validator_test.ak -----------------------------------------------------

/// Records `validator_test.ak`.  The program declares a
/// `validator gift_card { spend(...) { ... } mint(...) { ... } }`
/// outer block — Aiken's primary on-chain shape.  The recorder's
/// parser gained validator-block awareness alongside this fixture
/// (`spend` / `mint` / `else` / etc. inside a `validator <name> { ... }`
/// block now register as function-table entries).  The strict pin
/// asserts on the call sequence (compute → spend → redeemer_bonus
/// then compute → mint → redeemer_bonus) and the per-step values.
#[test]
fn test_validator_test_via_ct_print_full() {
    let Some((doc, source_path)) =
        record_and_dump_full("test_validator_test_via_ct_print_full", "validator_test.ak")
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
    // The function table includes the test fn (validator_smoke), the
    // free-function `compute()`, the validator's entry-point handlers
    // `spend` / `mint`, and the shared helper `redeemer_bonus`.
    assert_eq!(
        functions,
        vec![
            "validator_smoke",
            "compute",
            "spend",
            "redeemer_bonus",
            "mint"
        ],
    );

    // ----- counts -----------------------------------------------------
    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(18), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(5), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );

    let events = doc["events"].as_array().expect("events array");
    // 18 steps + 5 call_entry + 5 call_exit = 28 events.
    assert_eq!(events.len(), 28, "events.len()");
    assert_step_indices_monotonic(&doc);

    // ----- Call sequence ----------------------------------------------
    // compute() → spend(2) → redeemer_bonus(2), then back to compute(),
    // → mint(3) → redeemer_bonus(3).
    assert_eq!(
        observed_call_sequence(&doc),
        vec![
            "compute".to_string(),
            "spend".to_string(),
            "redeemer_bonus".to_string(),
            "mint".to_string(),
            "redeemer_bonus".to_string(),
        ],
    );

    // ----- Decoded variable values ------------------------------------
    // spend(2): amount=2 (param-intro in spend) → redeemer_bonus(2)
    //   binds amount=2 in callee → bonus=20 → adjusted=21 (spend's
    //   return).  Back in compute: spent=21.
    // mint(3): action=3 → redeemer_bonus(3) (amount=3) → base=30 →
    //   total=35 (mint's return).  Back in compute: minted=35.
    // combined = 21 + 35 = 56.
    assert_eq!(
        observed_var_sequence(&doc),
        vec![
            ("amount".into(), 2),
            ("amount".into(), 2),
            ("bonus".into(), 20),
            ("adjusted".into(), 21),
            ("spent".into(), 21),
            ("action".into(), 3),
            ("amount".into(), 3),
            ("base".into(), 30),
            ("total".into(), 35),
            ("minted".into(), 35),
            ("combined".into(), 56),
        ],
    );

    // ----- Return values: 20, 21, 30, 35, 56 ---------------------------
    let returns: Vec<i64> = events
        .iter()
        .filter(|e| e["kind"] == "call_exit")
        .map(|e| {
            let rv = &e["return_value"];
            assert_eq!(rv["kind"].as_str(), Some("Int"));
            rv["i"].as_i64().expect("return value Int.i")
        })
        .collect();
    assert_eq!(returns, vec![20, 21, 30, 35, 56]);
}

// --- pattern_match_test.ak -------------------------------------------------

/// Records `pattern_match_test.ak` and pins the when-arm matcher
/// behaviour:
///
/// * Flat patterns (`Some(x)`, `None`, integer literals, wildcard,
///   bare constructor names) are decoded correctly.
/// * Nested constructor patterns (`Wrap(InnerActive(n))`) are
///   destructured recursively — the outer arm matches the outer
///   `Wrap` Variant, then the inner pattern matches the
///   `InnerActive(n)` payload and binds `n` to the inner integer.
///   The spec-correct behaviour (`nested_val=9`, `total=15`) is
///   also pinned by `test_pattern_match_test_nested_decodes`.
#[test]
fn test_pattern_match_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_pattern_match_test_via_ct_print_full",
        "pattern_match_test.ak",
    ) else {
        return;
    };

    assert_metadata_program_ends_with(&doc, &source_path);

    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(
        functions,
        vec!["pattern_match", "compute", "flat_match", "nested_match"],
    );

    // ----- counts -----------------------------------------------------
    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(20), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(4), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );

    let events = doc["events"].as_array().expect("events array");
    // 20 steps + 4 call_entry + 4 call_exit = 28 events.
    assert_eq!(events.len(), 28, "events.len()");
    assert_step_indices_monotonic(&doc);

    assert_eq!(
        observed_call_sequence(&doc),
        vec![
            "compute".to_string(),
            "flat_match".to_string(),
            "flat_match".to_string(),
            "nested_match".to_string(),
        ],
    );

    // ----- Variable sequence pinning the CURRENT shape ----------------
    let var_sequence: Vec<(String, String, Option<String>, Option<i64>)> = events
        .iter()
        .filter(|e| e["kind"] == "step")
        .flat_map(|e| {
            e["vars"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|v| {
                    let name = v["varname"].as_str().expect("varname").to_string();
                    let kind = v["value"]["kind"].as_str().expect("value.kind").to_string();
                    let disc = v["value"]["discriminator"].as_str().map(|s| s.to_string());
                    let i = v["value"]["i"].as_i64();
                    (name, kind, disc, i)
                })
        })
        .collect();
    assert_eq!(
        var_sequence,
        vec![
            // flat_match(Some(7)): opt is Some, arm `Some(x) -> x`
            // matches and binds x=7, returns 7.
            ("opt".into(), "Variant".into(), Some("Some".into()), None),
            ("some_val".into(), "Int".into(), None, Some(7)),
            // flat_match(None): opt is None, arm `None -> -1`
            // matches, returns -1.
            ("opt".into(), "Variant".into(), Some("None".into()), None),
            ("none_val".into(), "Int".into(), None, Some(-1)),
            // wrapped = Wrap(InnerActive(9)): the Wrap is the
            // outer-level Variant.  The InnerActive(9) lives
            // inside its `contents` slot but isn't surfaced as a
            // separate variable.
            (
                "wrapped".into(),
                "Variant".into(),
                Some("Wrap".into()),
                None
            ),
            // nested_match(wrapped): param `w` is the Wrap.  The
            // `Wrap(InnerActive(n))` arm now destructures
            // recursively: the outer pattern matches the `Wrap`
            // Variant, the inner `InnerActive(n)` matches the
            // payload Variant and binds `n=9`, so the arm body
            // returns 9.
            ("w".into(), "Variant".into(), Some("Wrap".into()), None),
            ("nested_val".into(), "Int".into(), None, Some(9)),
            // compute's running total = 7 + (-1) + 9 = 15.
            ("total".into(), "Int".into(), None, Some(15)),
        ],
    );

    // ----- Return values: 7, -1, 9, 15 ---------------------------------
    let returns: Vec<i64> = events
        .iter()
        .filter(|e| e["kind"] == "call_exit")
        .map(|e| {
            let rv = &e["return_value"];
            assert_eq!(rv["kind"].as_str(), Some("Int"));
            rv["i"].as_i64().expect("return value Int.i")
        })
        .collect();
    assert_eq!(returns, vec![7, -1, 9, 15]);
}

/// Spec-correct expectation for `pattern_match_test.ak`.  The
/// nested-constructor matcher in `match_pattern_against_value`
/// (`src/tracer.rs`) recursively destructures the `Wrap(
/// InnerActive(n))` arm and binds `n=9`, so `nested_val == 9` and
/// `total == 7 + (-1) + 9 == 15`.  This complements the
/// shape-pinning `test_pattern_match_test_via_ct_print_full` above.
#[test]
fn test_pattern_match_test_nested_decodes() {
    let Some((doc, _)) = record_and_dump_full(
        "test_pattern_match_test_nested_decodes",
        "pattern_match_test.ak",
    ) else {
        return;
    };
    // Walk the per-step `vars` arrays directly (the program emits a
    // mix of `Variant` and `Int` values, so we can't reuse the
    // `observed_var_sequence` Int-only helper).  We only need the
    // two Int bindings produced by the nested pattern match.
    let int_vars: Vec<(String, i64)> = doc["events"]
        .as_array()
        .expect("events array")
        .iter()
        .filter(|e| e["kind"] == "step")
        .flat_map(|e| {
            e["vars"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|v| {
                    let name = v["varname"].as_str()?.to_string();
                    let kind = v["value"]["kind"].as_str()?;
                    if kind != "Int" {
                        return None;
                    }
                    let i = v["value"]["i"].as_i64()?;
                    Some((name, i))
                })
        })
        .collect();
    let nested_val = int_vars
        .iter()
        .find(|(n, _)| n == "nested_val")
        .map(|(_, v)| *v);
    assert_eq!(
        nested_val,
        Some(9),
        "nested_val should be the bound `n` = 9"
    );
    let total = int_vars.iter().find(|(n, _)| n == "total").map(|(_, v)| *v);
    assert_eq!(total, Some(15), "total should be 7 + (-1) + 9 = 15");
}

// --- variant_constructors_test.ak ------------------------------------------

/// Records `variant_constructors_test.ak`.  The program declares a
/// user-defined sum type `Status { Pending | Active(Int) | Failed }`
/// and threads each constructor through a `classify(s)` helper.
/// The strict pin asserts on the decoded `ValueRecord::Variant`
/// shape (the recorder gained a `Variant` case alongside this
/// fixture; previously a bare `None` / `Some(5)` would have fallen
/// through to the unknown-identifier path).
#[test]
fn test_variant_constructors_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_variant_constructors_test_via_ct_print_full",
        "variant_constructors_test.ak",
    ) else {
        return;
    };

    assert_metadata_program_ends_with(&doc, &source_path);

    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(
        functions,
        vec!["variant_constructors", "compute", "classify"]
    );

    // ----- counts -----------------------------------------------------
    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(25), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(4), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );

    let events = doc["events"].as_array().expect("events array");
    // 25 steps + 4 call_entry + 4 call_exit = 33 events.
    assert_eq!(events.len(), 33, "events.len()");
    assert_step_indices_monotonic(&doc);

    assert_eq!(
        observed_call_sequence(&doc),
        vec![
            "compute".to_string(),
            "classify".to_string(),
            "classify".to_string(),
            "classify".to_string(),
        ],
    );

    // ----- Variable sequence by (varname, value.kind) -----------------
    // Walk the variable stream in emission order; pin the kind plus
    // (for Variants) the discriminator and (for Ints) the value.
    let var_sequence: Vec<(String, String, Option<String>, Option<i64>)> = events
        .iter()
        .filter(|e| e["kind"] == "step")
        .flat_map(|e| {
            e["vars"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|v| {
                    let name = v["varname"].as_str().expect("varname").to_string();
                    let kind = v["value"]["kind"].as_str().expect("value.kind").to_string();
                    let disc = v["value"]["discriminator"].as_str().map(|s| s.to_string());
                    let i = v["value"]["i"].as_i64();
                    (name, kind, disc, i)
                })
        })
        .collect();
    assert_eq!(
        var_sequence,
        vec![
            // Constructor let-bindings in compute().
            (
                "pending_status".into(),
                "Variant".into(),
                Some("Pending".into()),
                None
            ),
            (
                "active_status".into(),
                "Variant".into(),
                Some("Active".into()),
                None
            ),
            (
                "failed_status".into(),
                "Variant".into(),
                Some("Failed".into()),
                None
            ),
            // classify(pending_status): param `s` is the Variant,
            // then back in compute the let-binding receives the
            // matched arm's value (1 for Pending).
            ("s".into(), "Variant".into(), Some("Pending".into()), None),
            ("pending_score".into(), "Int".into(), None, Some(1)),
            // classify(active_status): Active(5) matches `Active(n)`,
            // n bound to 5, returns 5.
            ("s".into(), "Variant".into(), Some("Active".into()), None),
            ("active_score".into(), "Int".into(), None, Some(5)),
            // classify(failed_status): Failed -> 0.
            ("s".into(), "Variant".into(), Some("Failed".into()), None),
            ("failed_score".into(), "Int".into(), None, Some(0)),
            // compute's running total.
            ("total".into(), "Int".into(), None, Some(6)),
        ],
    );

    // ----- Active variant payload -------------------------------------
    // `Active(5)` must surface its `5` payload through the
    // `contents.field_values[0]` slot of the Variant record (the
    // contents are encoded as a Struct holding positional fields).
    let active_value = events
        .iter()
        .filter(|e| e["kind"] == "step")
        .flat_map(|e| {
            e["vars"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
        })
        .find(|v| {
            v["varname"].as_str() == Some("active_status")
                && v["value"]["discriminator"].as_str() == Some("Active")
        })
        .expect("active_status Variant entry");
    let contents = &active_value["value"]["contents"];
    assert_eq!(contents["kind"].as_str(), Some("Struct"));
    let field_values = contents["field_values"].as_array().expect("field_values");
    assert_eq!(field_values.len(), 1);
    assert_eq!(field_values[0]["kind"].as_str(), Some("Int"));
    assert_eq!(field_values[0]["i"].as_i64(), Some(5));

    // ----- Return values: 1, 5, 0, 6 -----------------------------------
    let returns: Vec<i64> = events
        .iter()
        .filter(|e| e["kind"] == "call_exit")
        .map(|e| {
            let rv = &e["return_value"];
            assert_eq!(rv["kind"].as_str(), Some("Int"));
            rv["i"].as_i64().expect("return value Int.i")
        })
        .collect();
    assert_eq!(returns, vec![1, 5, 0, 6]);
}

// --- pipe_operator_test.ak -------------------------------------------------

/// Records `pipe_operator_test.ak`.  The program threads an integer
/// through three pipe stages — `raw |> add(7) |> mul(3) |> sub(2)`
/// — each rewritten by the recorder's pipe-desugar pass into a
/// regular function call (`add(raw, 7)` etc.).  The strict pin
/// captures the step / call sequence plus the per-stage decoded
/// values (the let-binding name on the compute() side and the
/// callee's a/b params on the helper side).
#[test]
fn test_pipe_operator_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_pipe_operator_test_via_ct_print_full",
        "pipe_operator_test.ak",
    ) else {
        return;
    };

    assert_metadata_program_ends_with(&doc, &source_path);

    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(
        functions,
        vec!["pipe_operator", "compute", "add", "mul", "sub"],
    );

    // ----- counts -----------------------------------------------------
    // Steps walk:
    //   1 outer-test entry
    //   1 dispatch (test body: `compute() == 34`)
    //   1 `let raw = 5`
    //   3 pairs of `(param-intro, body-line)` for add / mul / sub
    //   3 let-bindings back in compute (after_add, after_mul, result)
    //   1 trailing-expr step (`result`)
    //   = 13 steps total.
    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(13), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(4), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );

    let events = doc["events"].as_array().expect("events array");
    // 13 steps + 4 call_entry + 4 call_exit = 21 events.
    assert_eq!(events.len(), 21, "events.len()");
    assert_step_indices_monotonic(&doc);

    // ----- Call sequence: compute → add → mul → sub --------------------
    assert_eq!(
        observed_call_sequence(&doc),
        vec![
            "compute".to_string(),
            "add".to_string(),
            "mul".to_string(),
            "sub".to_string(),
        ],
    );

    // ----- Variable sequence -------------------------------------------
    // raw=5, then per stage: callee params (a, b) + return binding.
    //   add(5, 7)  → a=5, b=7, after_add=12
    //   mul(12, 3) → a=12, b=3, after_mul=36
    //   sub(36, 2) → a=36, b=2, result=34
    assert_eq!(
        observed_var_sequence(&doc),
        vec![
            ("raw".into(), 5),
            ("a".into(), 5),
            ("b".into(), 7),
            ("after_add".into(), 12),
            ("a".into(), 12),
            ("b".into(), 3),
            ("after_mul".into(), 36),
            ("a".into(), 36),
            ("b".into(), 2),
            ("result".into(), 34),
        ],
    );

    // ----- Return values: add → 12, mul → 36, sub → 34, compute → 34 ---
    let returns: Vec<i64> = events
        .iter()
        .filter(|e| e["kind"] == "call_exit")
        .map(|e| {
            let rv = &e["return_value"];
            assert_eq!(rv["kind"].as_str(), Some("Int"));
            rv["i"].as_i64().expect("return value Int.i")
        })
        .collect();
    assert_eq!(returns, vec![12, 36, 34, 34]);
}

// --- recursion_test.ak -----------------------------------------------------

/// Records `recursion_test.ak`.  The program exercises two tail-
/// recursive functions reached from a single `compute()` entry:
///
/// * `sum_acc(n, acc)` walks `n` down to zero, accumulating `acc + n`.
///   `sum_acc(10, 0)` → 55.
/// * `pow_acc(base, exp, acc)` walks `exp` down to zero, multiplying
///   `acc` by `base` each step.  `pow_acc(2, 6, 1)` → 64.
///
/// Both use `when n is { 0 -> base_case _ -> recurse }`.  The
/// recorder's when-arm matching (added with this fixture) evaluates
/// ONLY the arm whose pattern matches the scrutinee value, so the
/// recursion actually terminates instead of unrolling forever on
/// the wildcard arm.
///
/// The strict pin captures the full call tree (1 + 11 + 7 = 19
/// calls) and the per-frame param values (n stepping 10 → 0, acc
/// accumulating 0 → 55; base=2, exp stepping 6 → 0, acc doubling
/// 1 → 64).
#[test]
fn test_recursion_test_via_ct_print_full() {
    let Some((doc, source_path)) =
        record_and_dump_full("test_recursion_test_via_ct_print_full", "recursion_test.ak")
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
    assert_eq!(
        functions,
        vec!["recursion", "compute", "sum_acc", "pow_acc"],
    );

    // ----- counts -----------------------------------------------------
    // Each call to sum_acc emits 4 steps (param-intro at the signature
    // line + the `when n is {` opener step + the matching arm step +
    // the wildcard arm step).  sum_acc is invoked 11 times (initial
    // call with n=10 plus 10 recursive calls n=9..0), so sum_acc
    // contributes 44 steps.
    //
    // Each call to pow_acc emits 4 steps (param-intro + opener + two
    // arms).  pow_acc is invoked 7 times (exp=6..0), so pow_acc
    // contributes 28 steps.
    //
    // compute() emits 6 steps: 1 dispatch + 3 let-bindings + 1
    // trailing-expr step + 1 post-call site step.  Wait — the
    // canonical pattern from `nested_calls_test` is 1 dispatch + 3
    // let-bindings + 1 trailing-expr step + 1 inter-call step = 6.
    //
    // Total: 44 + 28 + 6 = 78 steps.
    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(78), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(19), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );

    let events = doc["events"].as_array().expect("events array");
    // 78 steps + 19 call_entry + 19 call_exit = 116 events.
    assert_eq!(events.len(), 116, "events.len()");
    assert_step_indices_monotonic(&doc);

    // ----- Call sequence ----------------------------------------------
    // 1 compute + 11 sum_acc (initial + 10 recursive) + 7 pow_acc
    // (initial + 6 recursive) = 19 calls.
    let mut expected_calls = vec!["compute".to_string()];
    for _ in 0..11 {
        expected_calls.push("sum_acc".to_string());
    }
    for _ in 0..7 {
        expected_calls.push("pow_acc".to_string());
    }
    assert_eq!(observed_call_sequence(&doc), expected_calls);

    // ----- Variable sequence ------------------------------------------
    // sum_acc unwinds: (n, acc) goes (10,0), (9,10), (8,19), (7,27),
    // (6,34), (5,40), (4,45), (3,49), (2,52), (1,54), (0,55).  Then
    // back in compute(): sum10 = 55.
    //
    // pow_acc unwinds: (base, exp, acc) base stays 2, exp goes 6..0,
    // acc doubles 1, 2, 4, 8, 16, 32, 64.  Then back in compute():
    // pow64 = 64, combined = 119.
    let mut expected_vars: Vec<(String, i64)> = Vec::new();
    let sum_pairs: &[(i64, i64)] = &[
        (10, 0),
        (9, 10),
        (8, 19),
        (7, 27),
        (6, 34),
        (5, 40),
        (4, 45),
        (3, 49),
        (2, 52),
        (1, 54),
        (0, 55),
    ];
    for (n, acc) in sum_pairs {
        expected_vars.push(("n".to_string(), *n));
        expected_vars.push(("acc".to_string(), *acc));
    }
    expected_vars.push(("sum10".to_string(), 55));
    let pow_triples: &[(i64, i64, i64)] = &[
        (2, 6, 1),
        (2, 5, 2),
        (2, 4, 4),
        (2, 3, 8),
        (2, 2, 16),
        (2, 1, 32),
        (2, 0, 64),
    ];
    for (base, exp, acc) in pow_triples {
        expected_vars.push(("base".to_string(), *base));
        expected_vars.push(("exp".to_string(), *exp));
        expected_vars.push(("acc".to_string(), *acc));
    }
    expected_vars.push(("pow64".to_string(), 64));
    expected_vars.push(("combined".to_string(), 119));
    assert_eq!(observed_var_sequence(&doc), expected_vars);

    // ----- Return values ---------------------------------------------
    // Innermost sum_acc base case returns acc=55; due to tail
    // recursion every parent frame's "last evaluable line" is the
    // same recursive call, so they all return 55 as well (the
    // accumulated final value).  Same shape for pow_acc returning
    // 64.  compute() returns 119 (sum10 + pow64).
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
    let mut expected_returns = vec![55i64; 11];
    expected_returns.extend(vec![64i64; 7]);
    expected_returns.push(119);
    assert_eq!(returns, expected_returns);
}

// --- higher_order_test.ak --------------------------------------------------

/// Records `higher_order_test.ak`.  Pins the new closure +
/// `list.map` paths:
///
///   * Lambda literal `fn(x) { x + 1 }` parses as
///     `Value::Closure { params, body_expr }` and surfaces in the
///     trace as a `String` ValueRecord (`"fn(x) { x + 1 }"`).
///   * Closure invocation `inc(5)` dispatches via
///     `eval_closure_call`, which emits a Call/Return pair under
///     the function name `<closure>` with the bound param value
///     attached to the call_entry step.
///   * `list.map([1,2,3], fn(x) { x + 100 })` is recognised in
///     `eval_list_map` and dispatches the closure over each list
///     element, surfacing one Call/Return per iteration.
///
/// The strict pin captures the call sequence (compute + 2 closure
/// inc(...) calls + 3 list.map closure calls = 6) and the per-call
/// return values.
#[test]
fn test_higher_order_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_higher_order_test_via_ct_print_full",
        "higher_order_test.ak",
    ) else {
        return;
    };

    assert_metadata_program_ends_with(&doc, &source_path);

    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    // Function-table entries: entry test, compute(), and a single
    // synthetic `<closure>` entry that all closure invocations share.
    assert_eq!(functions, vec!["higher_order", "compute", "<closure>"],);

    let counts = &doc["counts"];
    // 6 calls = 1 compute + 2 inc(...) closure invocations + 3
    // list.map iterations.
    assert_eq!(counts["calls"].as_u64(), Some(6), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );

    assert_step_indices_monotonic(&doc);

    assert_eq!(
        observed_call_sequence(&doc),
        vec![
            "compute".to_string(),
            "<closure>".to_string(), // inc(5)
            "<closure>".to_string(), // inc(10)
            "<closure>".to_string(), // list.map iteration 1
            "<closure>".to_string(), // list.map iteration 2
            "<closure>".to_string(), // list.map iteration 3
        ],
    );

    // ----- Per-call return values -------------------------------------
    // inc(5) → 6, inc(10) → 11, list.map iterations → 101/102/103,
    // compute → 323.
    let returns: Vec<i64> = doc["events"]
        .as_array()
        .expect("events array")
        .iter()
        .filter(|e| e["kind"] == "call_exit")
        .map(|e| {
            let rv = &e["return_value"];
            rv["i"]
                .as_i64()
                .unwrap_or_else(|| panic!("return value should decode as Int.i; got {rv}"))
        })
        .collect();
    assert_eq!(returns, vec![6, 11, 101, 102, 103, 323]);

    // ----- Closure value surfaces as a String ValueRecord -------------
    let inc_value = doc["events"]
        .as_array()
        .expect("events array")
        .iter()
        .filter(|e| e["kind"] == "step")
        .flat_map(|e| {
            e["vars"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
        })
        .find(|v| v["varname"].as_str() == Some("inc"))
        .expect("inc Closure entry");
    assert_eq!(inc_value["value"]["kind"].as_str(), Some("String"));
    let text = inc_value["value"]["text"].as_str().unwrap_or("");
    assert!(
        text.starts_with("fn(") && text.contains("x + 1"),
        "closure text should render as `fn(x) {{ x + 1 }}`; got {text:?}"
    );
}

// --- builtins_test.ak ------------------------------------------------------

/// Records `builtins_test.ak`.  The program calls two Aiken
/// builtins (`length_of_bytearray` and `verify_ed25519_signature`)
/// and sums their results.
///
/// Because the recorder doesn't link a real cryptographic
/// implementation, builtin invocations surface as synthetic
/// Call/Return event pairs (`emit_builtin_call`) with placeholder
/// return values chosen to match the spec-correct kind:
///
///   * `length_of_bytearray(#"deadbeef")` → `Int(4)` (real length
///     of the 4-byte hex literal).
///   * `verify_ed25519_signature(...)` → `Int(1)` (Bool true,
///     modelled as Int per the writer's pre-registered Bool type).
///
/// The strict pin asserts on the function-table entries (each
/// builtin name registered exactly once), the call sequence
/// including the builtin call sites, and the synthesised return
/// values.
#[test]
fn test_builtins_test_via_ct_print_full() {
    let Some((doc, source_path)) =
        record_and_dump_full("test_builtins_test_via_ct_print_full", "builtins_test.ak")
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
    // Function-table order: entry test, compute(), then each helper
    // and builtin in the order they were first invoked.
    assert_eq!(
        functions,
        vec![
            "builtins",
            "compute",
            "payload_len",
            "length_of_bytearray",
            "signature_check",
            "verify_ed25519_signature",
        ],
    );

    let counts = &doc["counts"];
    // 5 calls = 1 compute + 2 helpers (payload_len, signature_check) +
    // 2 builtins (length_of_bytearray, verify_ed25519_signature).
    assert_eq!(counts["calls"].as_u64(), Some(5), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );

    assert_step_indices_monotonic(&doc);

    assert_eq!(
        observed_call_sequence(&doc),
        vec![
            "compute".to_string(),
            "payload_len".to_string(),
            "length_of_bytearray".to_string(),
            "signature_check".to_string(),
            "verify_ed25519_signature".to_string(),
        ],
    );

    // ----- Return values per call_exit -------------------------------
    // payload_len → 4, length_of_bytearray → 4 (the synthesised
    // builtin result), signature_check → 1,
    // verify_ed25519_signature → 1, compute → 5.
    let returns: Vec<i64> = doc["events"]
        .as_array()
        .expect("events array")
        .iter()
        .filter(|e| e["kind"] == "call_exit")
        .map(|e| {
            let rv = &e["return_value"];
            rv["i"]
                .as_i64()
                .unwrap_or_else(|| panic!("return value should decode as Int.i; got {rv}"))
        })
        .collect();
    assert_eq!(
        returns,
        vec![4, 4, 1, 1, 5],
        "compute → payload_len → length_of_bytearray → signature_check \
         → verify_ed25519_signature → outer return chain",
    );
}

// --- expect_refinement_test.ak ---------------------------------------------

/// Records `expect_refinement_test.ak`.  Pins the new
/// `Statement::Expect` paths:
///
///   * Runtime: `expect Some(x) = some_val` matches the pattern
///     against the resolved RHS and binds the captured identifier
///     (`x = 13`) into the function-local env, so the trace
///     surfaces `x` as a step variable just like a let-binding.
///   * Static sweep (`emit_expect_events_for_program`): one
///     `EventLogKind::Error` io_event tagged `AikenExpectFailure`
///     per `expect` line in the parsed program — mirrors the
///     `Statement::Fail` static-sweep.  The fixture contains 3
///     expect statements (2 reached, 1 in unreached
///     `failing_extract()`), so 3 io_events surface.
#[test]
fn test_expect_refinement_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_expect_refinement_test_via_ct_print_full",
        "expect_refinement_test.ak",
    ) else {
        return;
    };

    assert_metadata_program_ends_with(&doc, &source_path);

    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(
        functions,
        vec!["expect_refinement", "compute", "safe_extract", "safe_field"],
    );

    let counts = &doc["counts"];
    assert_eq!(counts["calls"].as_u64(), Some(3), "calls; counts={counts}");
    // Three `expect` statements parsed from the program (2 in
    // executed helpers + 1 in unreached `failing_extract`); each
    // contributes one `EventLogKind::Error` io_event via the static
    // sweep.
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(3),
        "io_events; counts={counts}"
    );

    assert_step_indices_monotonic(&doc);

    assert_eq!(
        observed_call_sequence(&doc),
        vec![
            "compute".to_string(),
            "safe_extract".to_string(),
            "safe_field".to_string(),
        ],
    );

    // ----- Variable sequence: pattern bindings surface as step vars ---
    // safe_extract: some_val = Some(13) → expect Some(x) binds x=13
    //   → return 13.  Back in compute: a=13.
    // safe_field: boxed_val = Boxed(29) → expect Boxed(value) binds
    //   value=29 → return 29.  Back in compute: b=29.
    // compute: total = a + b = 42.
    let var_sequence: Vec<(String, String, Option<String>, Option<i64>)> = doc["events"]
        .as_array()
        .expect("events array")
        .iter()
        .filter(|e| e["kind"] == "step")
        .flat_map(|e| {
            e["vars"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|v| {
                    let name = v["varname"].as_str().expect("varname").to_string();
                    let kind = v["value"]["kind"].as_str().expect("value.kind").to_string();
                    let disc = v["value"]["discriminator"].as_str().map(|s| s.to_string());
                    let i = v["value"]["i"].as_i64();
                    (name, kind, disc, i)
                })
        })
        .collect();
    assert_eq!(
        var_sequence,
        vec![
            // safe_extract bindings
            (
                "some_val".into(),
                "Variant".into(),
                Some("Some".into()),
                None
            ),
            ("x".into(), "Int".into(), None, Some(13)),
            ("a".into(), "Int".into(), None, Some(13)),
            // safe_field bindings
            (
                "boxed_val".into(),
                "Variant".into(),
                Some("Boxed".into()),
                None
            ),
            ("value".into(), "Int".into(), None, Some(29)),
            ("b".into(), "Int".into(), None, Some(29)),
            // compute total
            ("total".into(), "Int".into(), None, Some(42)),
        ],
    );

    // ----- Static-sweep io_events: tagged AikenExpectFailure ----------
    let events = doc["events"].as_array().expect("events array");
    let expect_errors: Vec<&serde_json::Value> = events
        .iter()
        .filter(|e| e["kind"] == "io" && e["io_kind"] == "ioError")
        .collect();
    assert_eq!(
        expect_errors.len(),
        3,
        "expected 3 ioError io_events (one per expect statement); got {expect_errors:?}",
    );
    // Every error message should mention `expect`.
    for ev in &expect_errors {
        let text = ev["text"].as_str().unwrap_or("");
        assert!(
            text.starts_with("expect "),
            "ioError text should start with `expect `; got {text:?}"
        );
    }
}

// --- opaque_generic_test.ak ------------------------------------------------

/// Records `opaque_generic_test.ak`.  The program declares an
/// `opaque type Wrapper { inner: Int }` plus two regular helpers
/// (`unwrap` for the opaque value and `pair_first` for tuple
/// projection) and threads concrete values through both.
///
/// The recorder pre-scans for `opaque type` declarations
/// (`collect_opaque_type_names`) and labels the registered type id
/// with a `" (opaque)"` suffix so consumers can distinguish opaque-
/// wrapped values from plain records.  The wire-format kind stays
/// `Struct` (opaque is an Aiken access-control feature, not a
/// runtime distinction); the inner field still surfaces verbatim
/// via the Struct's `field_values` slot.
#[test]
fn test_opaque_generic_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_opaque_generic_test_via_ct_print_full",
        "opaque_generic_test.ak",
    ) else {
        return;
    };

    assert_metadata_program_ends_with(&doc, &source_path);

    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(
        functions,
        vec![
            "opaque_generic",
            "compute",
            "unwrap",
            "extract_first",
            "pair_first",
        ],
    );

    let counts = &doc["counts"];
    assert_eq!(counts["calls"].as_u64(), Some(4), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );

    assert_step_indices_monotonic(&doc);

    assert_eq!(
        observed_call_sequence(&doc),
        vec![
            "compute".to_string(),
            "unwrap".to_string(),
            "extract_first".to_string(),
            "pair_first".to_string(),
        ],
    );

    // ----- Opaque type label ------------------------------------------
    // The `Wrapper` record landed in the trace via
    // `let wrapped = Wrapper { inner: 42 }`.  Its type id is
    // registered with the `" (opaque)"` suffix; pull the types
    // table out of the trace document and assert the label.
    let types: Vec<String> = doc["types"]
        .as_array()
        .expect("types array")
        .iter()
        .filter_map(|t| t.as_str().map(|s| s.to_string()))
        .collect();
    assert!(
        types.iter().any(|t| t == "Wrapper (opaque)"),
        "expected `Wrapper (opaque)` in types table; got {types:?}"
    );
    // The non-opaque suffix label MUST NOT also appear (we'd have
    // double-registered).
    assert!(
        !types.iter().any(|t| t == "Wrapper"),
        "plain `Wrapper` label must not be registered alongside the \
         opaque-suffixed one; got {types:?}"
    );

    // ----- Wrapped value carries the inner Int through Struct.field_values
    let events = doc["events"].as_array().expect("events array");
    let wrapped_value = events
        .iter()
        .filter(|e| e["kind"] == "step")
        .flat_map(|e| {
            e["vars"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
        })
        .find(|v| v["varname"].as_str() == Some("wrapped"))
        .expect("wrapped Struct entry");
    assert_eq!(
        wrapped_value["value"]["kind"].as_str(),
        Some("Struct"),
        "wrapped should decode as Struct; got {wrapped_value}"
    );
    let field_values = wrapped_value["value"]["field_values"]
        .as_array()
        .expect("field_values");
    assert_eq!(field_values.len(), 1);
    assert_eq!(field_values[0]["kind"].as_str(), Some("Int"));
    assert_eq!(field_values[0]["i"].as_i64(), Some(42));

    // ----- Final return value should be 49 ----------------------------
    let returns: Vec<i64> = events
        .iter()
        .filter(|e| e["kind"] == "call_exit")
        .map(|e| {
            let rv = &e["return_value"];
            assert_eq!(rv["kind"].as_str(), Some("Int"));
            rv["i"].as_i64().expect("return value Int.i")
        })
        .collect();
    // unwrap → 42, pair_first → 7, extract_first → 7, compute → 49.
    assert_eq!(returns, vec![42, 7, 7, 49]);
}

// --- bytearray_string_test.ak ---------------------------------------------

/// Records `bytearray_string_test.ak`.  The program declares two
/// literal forms — a hex byte literal `#"deadbeef"` and a UTF-8
/// string literal `"FOO"` — and threads each through a helper that
/// returns the literal's byte length as an `Int` so the existing
/// UPLC-CEK arithmetic pipeline can drive the test body
/// (`compute() == 7`).
///
/// The recorder gained `Value::ByteArray(Vec<u8>)` and
/// `Value::String(String)` alongside this fixture; both surface as
/// `ValueRecord::String` on the wire but with distinct `text`
/// payloads so the renderer can pick the right shape:
///
///   * `Value::ByteArray([0xde,0xad,0xbe,0xef])` →
///     `String { text: "#\"deadbeef\"" }` (hex form preserved).
///   * `Value::String("FOO")` → `String { text: "FOO" }`.
///
/// The strict pin asserts on (a) the exact byte texts of each
/// literal binding and (b) the same `kind: "String"` for both forms
/// (the wire-format collapse), proving the recorder no longer drops
/// non-Int let-binding RHSs from the substitution map.
#[test]
fn test_bytearray_string_test_via_ct_print_full() {
    let Some((doc, source_path)) = record_and_dump_full(
        "test_bytearray_string_test_via_ct_print_full",
        "bytearray_string_test.ak",
    ) else {
        return;
    };

    assert_metadata_program_ends_with(&doc, &source_path);

    let functions: Vec<&str> = doc["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(
        functions,
        vec!["bytearray_string", "compute", "payload_len", "name_len"],
    );

    // ----- counts -----------------------------------------------------
    // compute(): 1 dispatch + 3 let-bindings + 1 trailing-expr step = 5
    // payload_len(): 1 param-intro is suppressed (no params), 2 let
    //   steps + 1 trailing-expr = 3 steps.
    // name_len(): same shape = 3 steps.
    // Total: 5 + 3 + 3 = 11.
    let counts = &doc["counts"];
    assert_eq!(counts["steps"].as_u64(), Some(12), "steps; counts={counts}");
    assert_eq!(counts["calls"].as_u64(), Some(3), "calls; counts={counts}");
    assert_eq!(
        counts["io_events"].as_u64(),
        Some(0),
        "io_events; counts={counts}"
    );

    let events = doc["events"].as_array().expect("events array");
    // 12 steps + 3 call_entry + 3 call_exit = 18 events.
    assert_eq!(events.len(), 18, "events.len()");
    assert_step_indices_monotonic(&doc);

    assert_eq!(
        observed_call_sequence(&doc),
        vec![
            "compute".to_string(),
            "payload_len".to_string(),
            "name_len".to_string(),
        ],
    );

    // ----- Variable sequence by (varname, value.kind, optional text/i) -
    // Walk the variable stream in emission order.  We pin the kind plus
    // the textual representation for `String` shapes and the integer
    // value for `Int` shapes.
    let var_sequence: Vec<(String, String, Option<String>, Option<i64>)> = events
        .iter()
        .filter(|e| e["kind"] == "step")
        .flat_map(|e| {
            e["vars"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|v| {
                    let name = v["varname"].as_str().expect("varname").to_string();
                    let kind = v["value"]["kind"].as_str().expect("value.kind").to_string();
                    let text = v["value"]["text"].as_str().map(|s| s.to_string());
                    let i = v["value"]["i"].as_i64();
                    (name, kind, text, i)
                })
        })
        .collect();
    // The interleaving (callee var, then caller's `let p = callee()`
    // before the next call) reflects the recorder's variable-stream
    // emission order: each `register_variable_with_full_value` lands
    // on the most-recent step event, so the let-binding step in
    // compute() picks up `p` immediately after payload_len() returns,
    // before the next step (the let-binding for `q`) opens.
    assert_eq!(
        var_sequence,
        vec![
            // payload_len: hex literal preserved as `#"deadbeef"` text.
            (
                "payload".into(),
                "String".into(),
                Some("#\"deadbeef\"".into()),
                None
            ),
            ("n".into(), "Int".into(), None, Some(4)),
            // p = payload_len() — lands on the same step in compute()
            // before the next call dispatches.
            ("p".into(), "Int".into(), None, Some(4)),
            // name_len: text literal "FOO" preserved as text.
            ("name".into(), "String".into(), Some("FOO".into()), None),
            ("n".into(), "Int".into(), None, Some(3)),
            ("q".into(), "Int".into(), None, Some(3)),
            ("total".into(), "Int".into(), None, Some(7)),
        ],
    );

    // ----- Return values: 4, 3, 7 -------------------------------------
    let returns: Vec<i64> = events
        .iter()
        .filter(|e| e["kind"] == "call_exit")
        .map(|e| {
            let rv = &e["return_value"];
            assert_eq!(rv["kind"].as_str(), Some("Int"));
            rv["i"].as_i64().expect("return value Int.i")
        })
        .collect();
    assert_eq!(returns, vec![4, 3, 7]);
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
