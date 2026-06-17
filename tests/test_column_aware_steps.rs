//! Column-aware replay-navigation regression test for the Cardano /
//! Aiken recorder (FU-Column-Aware-Nav-Cardano).
//!
//! Mirrors the JS recorder's
//! `tests/integration/column-aware.test.ts` "trace surfaces a column
//! field" fixture and matches the EVM (`tests/test_column_aware.rs`),
//! Solana (`tests/test_column_aware_steps.rs`), Cairo
//! (`tests/test_column_aware.rs`), Ruby
//! (`test/test_column_aware.rb`), and WASM ("flag is set + at least
//! one column-bearing step lands") sibling tests.
//!
//! The Aiken hand-rolled parser is line-oriented (one statement per
//! line) so this test takes the WASM-style assertion shape: it does
//! NOT require multiple statements on a single source line, it only
//! requires that the trace metadata advertises column-aware support
//! and that at least one emitted step carries a non-null `column`
//! field (the column equal to the first non-whitespace byte of the
//! statement's source line).  When the parser later gains multi-
//! statement-per-line support, this fixture can be re-anchored on
//! distinct columns within a single line — matching the Cairo /
//! EVM assertion shape.
//!
//! See `codetracer-specs/Planned-Features/
//! Column-Aware-Navigation-Other-Languages.plan.md` for the acceptance
//! criteria.

use std::path::{Path, PathBuf};
use std::process::Command;

const CTFS_MAGIC: [u8; 5] = [0xC0, 0xDE, 0x72, 0xAC, 0xE2];

/// Path to the `ct-print` binary shipped with `codetracer-trace-format-nim`.
fn ct_print_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("codetracer-trace-format-nim")
        .join(format!("ct-print{}", std::env::consts::EXE_SUFFIX))
}

fn ct_files_in(out_dir: &Path) -> Vec<PathBuf> {
    if !out_dir.exists() {
        return Vec::new();
    }
    std::fs::read_dir(out_dir)
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "ct"))
        .collect()
}

/// Returns the path to ct-print or logs a `SKIP:` diagnostic and
/// returns `None`.  Mirrors the convention enforced by
/// `tests/verify-cli-convention-no-silent-skip.sh`.
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

#[test]
fn test_column_aware_steps_flag_and_columns_emitted() {
    let test_name = "test_column_aware_steps_flag_and_columns_emitted";
    let Some(ct_print) = ct_print_or_skip(test_name) else {
        return;
    };

    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("test-programs")
        .join("aiken")
        .join("column_aware_test.ak");
    assert!(source.exists(), "fixture missing: {}", source.display());

    let tmp_dir = tempfile::tempdir().expect("tempdir");
    let out_dir = tmp_dir.path().join("traces");
    std::fs::create_dir_all(&out_dir).unwrap();

    codetracer_cardano_recorder::recorder::record(&source, &out_dir)
        .expect("recorder::record should succeed on column_aware_test.ak");

    // --- Sanity: the recorder produced a real CTFS container ---
    let ct_files = ct_files_in(&out_dir);
    assert!(
        !ct_files.is_empty(),
        "expected a .ct container in {out_dir:?}"
    );
    let ct_path = &ct_files[0];
    let content = std::fs::read(ct_path).expect("read .ct");
    assert!(content.len() >= 5, ".ct too small");
    assert_eq!(
        &content[..5],
        &CTFS_MAGIC,
        "CTFS magic bytes mismatch — recorder must produce a canonical CTFS container",
    );

    // --- Pipe the .ct through `ct-print --full` for JSON inspection ---
    let dump = Command::new(&ct_print)
        .args(["--full", "--strip-paths"])
        .arg(ct_path)
        .output()
        .expect("failed to run ct-print --full");
    assert!(
        dump.status.success(),
        "ct-print --full should succeed; stderr: {}",
        String::from_utf8_lossy(&dump.stderr),
    );

    let doc: serde_json::Value =
        serde_json::from_slice(&dump.stdout).expect("ct-print --full should emit valid JSON");

    // --- meta.dat bit 4: FLAG_HAS_COLUMN_AWARE_STEPS ------------------
    //
    // The trace metadata must advertise column-aware support so
    // downstream tooling knows to surface columns to the user.  Mirrors
    // the JS reference assertion at
    // `codetracer-js-recorder/tests/integration/column-aware.test.ts`.
    let has_column_aware = doc["metadata"]["flags"]["has_column_aware_steps"].as_bool();
    assert_eq!(
        has_column_aware,
        Some(true),
        "trace metadata must advertise has_column_aware_steps=true; got {:?}",
        doc["metadata"]
    );

    // --- At least one column-bearing step landed ----------------------
    //
    // The fixture's `compute()` body has three `let` bindings at
    // distinct indentation levels (columns 3, 5, 7).  Each must
    // surface as a step event carrying its source-byte column on the
    // wire.  We gather every emitted step's column and require:
    //
    //   * At least one step carries a non-null `column` field
    //     (proves `register_step_with_column` runs end-to-end).
    //   * Every surfaced column is >= 1 (the 1-based wire contract).
    //   * At least two distinct columns appear in the trace, proving
    //     the column resolution isn't a constant.
    let events = doc["events"].as_array().expect("events array");
    let mut columns: Vec<i64> = Vec::new();
    let mut steps_seen = 0usize;
    for ev in events {
        if ev["kind"] != "step" {
            continue;
        }
        steps_seen += 1;
        if let Some(col) = ev["column"].as_i64() {
            columns.push(col);
        }
    }
    assert!(
        steps_seen >= 3,
        "expected at least 3 step events from the fixture's three let bindings; \
         got {steps_seen}; events={events:?}",
    );
    assert!(
        !columns.is_empty(),
        "expected at least one step event with a non-null `column` field — \
         steps_seen={steps_seen}; events={events:?}",
    );
    for col in &columns {
        assert!(
            *col >= 1,
            "step column must be >= 1 (1-based on the wire); got {col}",
        );
    }
    let distinct: std::collections::BTreeSet<_> = columns.iter().copied().collect();
    assert!(
        distinct.len() >= 2,
        "expected the indented fixture to surface >= 2 distinct columns; \
         got {distinct:?}; full columns={columns:?}",
    );

    eprintln!(
        "PASS: column-aware step emission — {} steps with columns, distinct={:?}",
        columns.len(),
        distinct,
    );
}
