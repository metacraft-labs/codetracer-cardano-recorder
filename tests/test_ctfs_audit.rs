//! CTFS audit regression tests for the Cardano/Aiken recorder.
//!
//! These tests lock in the fixes landed during the 2026-05 CTFS audit
//! (entry 1.48 in `/tmp/isonim-migration.txt`).  They mirror the pattern
//! established by the EVM (1.39), Solana (1.44) and Move (1.46) recorder
//! audits.
//!
//! Each test corresponds to one bullet from the section 5.6 audit
//! checklist:
//!
//!  - (c) IO / structured events via `register_special_event` —
//!    `test_uplc_eval_error_emits_special_event`.
//!  - (e) Step records emitted — `test_steps_emitted_for_let_bindings`.
//!  - (f) Canonical CTFS schema match — `test_ctfs_format_advertised_in_help`
//!    and `test_ctfs_writer_produces_ct_container`.

use std::path::Path;

use codetracer_trace_writer_nim::{NimTraceReaderHandle, TraceEventsFileFormat};

const CTFS_MAGIC: [u8; 5] = [0xC0, 0xDE, 0x72, 0xAC, 0xE2];

/// Helper: build a tempdir, run the recorder against `flow_test.ak`, and
/// return the path to the produced trace container.
fn record_flow_test(format: TraceEventsFileFormat) -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp_dir = tempfile::tempdir().expect("tempdir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("test-programs/aiken/flow_test.ak");

    codetracer_cardano_recorder::recorder::record(&source_path, &out_dir, format)
        .expect("recorder::record should succeed");

    (tmp_dir, out_dir)
}

fn first_ct_file(out_dir: &Path) -> std::path::PathBuf {
    let mut ct_files: Vec<_> = std::fs::read_dir(out_dir)
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "ct"))
        .collect();
    ct_files.sort();
    assert!(
        !ct_files.is_empty(),
        "expected a .ct container in {:?}",
        out_dir
    );
    ct_files[0].clone()
}

fn open_ctfs_reader(out_dir: &Path) -> NimTraceReaderHandle {
    let ct_path = first_ct_file(out_dir);
    NimTraceReaderHandle::open(&ct_path.to_string_lossy())
        .unwrap_or_else(|e| panic!("failed to open Nim CTFS reader for {ct_path:?}: {e}"))
}

fn json_bytes_as_string(value: &serde_json::Value) -> String {
    let bytes: Vec<u8> = value
        .as_array()
        .unwrap_or_else(|| panic!("expected byte array JSON, got {value:#}"))
        .iter()
        .map(|byte| {
            byte.as_u64()
                .unwrap_or_else(|| panic!("expected byte value, got {byte:#}")) as u8
        })
        .collect();
    String::from_utf8(bytes).expect("event data should be UTF-8")
}

// ---- Audit (f): CTFS multi-stream container is producible -----------------

/// The recorder must be able to emit a canonical CTFS multi-stream `.ct`
/// container (the format that `NimTraceReaderHandle` and the db-backend
/// `CTFSTraceReader` consume directly).  Pre-fix, the CLI's
/// `OutputFormat` enum did not even expose `Ctfs` — only `Binary` /
/// `Json` — so there was no way to request the canonical container.
///
/// This is a runtime smoke test that the writer accepts
/// `TraceEventsFileFormat::Ctfs` and the produced `.ct` file starts with
/// the canonical CTFS magic bytes.
#[test]
fn test_ctfs_writer_produces_ct_container() {
    let (_tmp, out_dir) = record_flow_test(TraceEventsFileFormat::Ctfs);

    let ct_path = first_ct_file(&out_dir);
    let content = std::fs::read(&ct_path).expect("read ct file");
    assert!(content.len() >= 5, ".ct container too small");
    assert_eq!(
        &content[..5],
        &CTFS_MAGIC,
        "produced container should start with CTFS magic bytes"
    );
}

/// The CLI binary must accept `ctfs` as a `--format` value AND default
/// to it.  This catches accidental regressions in the CLI surface (e.g.
/// someone reverting the `OutputFormat` enum back to the pre-fix
/// `Binary` / `Json` only shape).
///
/// Same shape as the Move (1.46) and Solana (1.44) audit smoke tests.
#[test]
fn test_ctfs_format_advertised_in_help() {
    use std::process::Command;

    let bin = env!("CARGO_BIN_EXE_codetracer-cardano-recorder");
    let output = Command::new(bin)
        .args(["record", "--help"])
        .output()
        .expect("failed to run codetracer-cardano-recorder record --help");

    assert!(output.status.success(), "--help should exit 0");

    let help = String::from_utf8_lossy(&output.stdout);
    assert!(
        help.contains("ctfs"),
        "`record --help` should advertise `ctfs` as a --format value; got:\n{help}"
    );
    assert!(
        help.contains("[default: ctfs]"),
        "`record --help` should default --format to `ctfs`; got:\n{help}"
    );
}

// ---- Audit (e): Step records emitted on every line transition -------------

/// `register_step` must fire on each LetBinding / Expr statement so the
/// frontend can step line-by-line through the Aiken source.  The assertion
/// opens the produced `.ct` with the Nim reader and verifies the reader can
/// see the expected top-level/compute calls, source path, and statement
/// steps instead of only checking for non-empty bytes.
#[test]
fn test_steps_emitted_for_let_bindings() {
    let (_tmp, out_dir) = record_flow_test(TraceEventsFileFormat::Ctfs);
    let reader = open_ctfs_reader(&out_dir);

    assert!(
        reader.step_count() >= 7,
        "flow_test.ak should expose statement steps through the CTFS reader"
    );
    assert!(
        reader.call_count() >= 1,
        "expected at least one readable call record"
    );

    let function_names: Vec<_> = (0..reader.function_count())
        .map(|id| reader.function(id).expect("read function name"))
        .collect();
    assert!(
        function_names.iter().any(|name| name == "flow_test"),
        "missing flow_test function in {function_names:#?}"
    );
    assert!(
        function_names.iter().any(|name| name == "compute"),
        "missing compute function in {function_names:#?}"
    );

    let paths: Vec<_> = (0..reader.path_count())
        .map(|id| reader.path(id).expect("read path"))
        .collect();
    assert!(
        paths.iter().any(|path| path.ends_with("flow_test.ak")),
        "missing flow_test.ak path in {paths:#?}"
    );

    let first_step: serde_json::Value =
        serde_json::from_str(&reader.step_json(0).expect("read first step JSON"))
            .expect("parse first step JSON");
    assert!(
        first_step["global_line_index"].as_u64().is_some(),
        "step JSON should expose a global line index: {first_step:#}"
    );
}

// ---- Audit (c): UPLC eval errors routed via register_special_event --------

/// When UPLC CEK evaluation fails (budget exhaustion, divide-by-zero,
/// type errors), the recorder must emit a `register_special_event` so
/// the failure surfaces in CodeTracer's event-log pane and the trace
/// finalises cleanly.  Pre-fix, eval errors propagated up via `?` and
/// aborted `trace_program` mid-write — leaving a partial trace and
/// dropping the failure message entirely.
///
/// We exercise this by writing a synthesised Aiken source whose final
/// expression triggers a divide-by-zero — UPLC's `divideInteger` raises
/// a CEK error on division by zero.  The recorder must complete
/// successfully (no error returned), produce a CTFS container, and expose
/// a readable error event through the Nim reader.
#[test]
fn test_uplc_eval_error_does_not_abort_trace() {
    let tmp_dir = tempfile::tempdir().expect("tempdir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    // Write a synthesised Aiken program that triggers div-by-zero in
    // UPLC.  The recorder's expression compiler maps `/` to UPLC
    // `divideInteger`, which fails on b=0.
    let source = "fn main() -> Int {\n  let a = 10\n  let b = 0\n  let c = a / b\n  c\n}\n";
    let source_path = tmp_dir.path().join("divzero.ak");
    std::fs::write(&source_path, source).expect("write source");

    // Pre-fix: this would propagate the UPLC error and abort the trace.
    // Post-fix: the eval error is routed through `register_special_event`
    // and the recorder finishes the trace cleanly.
    let result = codetracer_cardano_recorder::recorder::record(
        &source_path,
        &out_dir,
        TraceEventsFileFormat::Ctfs,
    );

    assert!(
        result.is_ok(),
        "post-fix: divide-by-zero should be captured as a special event, \
         not abort the recorder.  Got: {result:?}"
    );

    let reader = open_ctfs_reader(&out_dir);
    assert!(
        reader.event_count() > 0,
        "expected a readable CTFS error event after eval failure"
    );

    let events: Vec<_> = (0..reader.event_count())
        .map(|idx| reader.event_json(idx).expect("read event JSON"))
        .collect();
    let error_contents: Vec<_> = events
        .iter()
        .map(|event| serde_json::from_str::<serde_json::Value>(event).expect("parse event JSON"))
        .filter(|event| event["kind"] == "error")
        .map(|event| json_bytes_as_string(&event["data"]))
        .collect();

    assert!(
        error_contents
            .iter()
            .any(|content| content.to_lowercase().contains("divide")),
        "expected divide-by-zero error content in readable CTFS events; events={events:#?}"
    );
}
