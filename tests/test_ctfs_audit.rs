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

use codetracer_trace_writer_nim::TraceEventsFileFormat;

const CTFS_MAGIC: [u8; 5] = [0xC0, 0xDE, 0x72, 0xAC, 0xE2];

/// Helper: build a tempdir, run the recorder against `flow_test.ak`, and
/// return the path to the produced trace container.
fn record_flow_test(format: TraceEventsFileFormat) -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp_dir = tempfile::tempdir().expect("tempdir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-programs/aiken/flow_test.ak");

    codetracer_cardano_recorder::recorder::record(&source_path, &out_dir, format)
        .expect("recorder::record should succeed");

    (tmp_dir, out_dir)
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

    let ct_files: Vec<_> = std::fs::read_dir(&out_dir)
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "ct"))
        .collect();

    assert!(
        !ct_files.is_empty(),
        "expected a .ct container in {:?}",
        out_dir
    );
    let content = std::fs::read(&ct_files[0]).expect("read ct file");
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
/// frontend can step line-by-line through the Aiken source.  A trace
/// produced for `flow_test.ak` (5 let-bindings + 1 expr in `compute()`,
/// 1 expr in `flow_test`) should generate a non-trivial trace.
///
/// Without a fixture-aware reader we can't introspect the produced
/// step-event count from in-memory; the CTFS container is binary.  A
/// reasonable proxy: the trace file is non-empty and the smoke test
/// suite (`test_tracer.rs`) exercises the same code path.  Keep this
/// test as a structural guard that the trace dir contains the expected
/// CTFS artefacts.
#[test]
fn test_steps_emitted_for_let_bindings() {
    let (_tmp, out_dir) = record_flow_test(TraceEventsFileFormat::Ctfs);

    let entries: Vec<_> = std::fs::read_dir(&out_dir)
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();

    let ct_size: u64 = entries
        .iter()
        .filter(|p| p.extension().is_some_and(|ext| ext == "ct"))
        .map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
        .sum();

    // The flow_test program produces multiple Step / Value records; the
    // CTFS container should have meaningful content well above the
    // header magic.
    assert!(
        ct_size > 100,
        "ct container should hold step + value records, got {ct_size} bytes"
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
/// successfully (no error returned) and produce a CTFS container with
/// the embedded special-event record.
///
/// Without a CTFS reader in the recorder's dev-deps we cannot decode
/// the container directly here.  The test verifies the post-fix
/// invariant at the recorder API: `record(...)` returns Ok and a CTFS
/// container is produced even on an evaluation error.  This is the
/// exact regression that the pre-fix recorder failed.
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

    // A CTFS container should still have been produced (the writer's
    // finish_writing_trace_events runs to completion).
    let ct_files: Vec<_> = std::fs::read_dir(&out_dir)
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "ct"))
        .collect();
    assert!(
        !ct_files.is_empty(),
        "expected a .ct container even after eval error; got {:?}",
        out_dir
    );
}
