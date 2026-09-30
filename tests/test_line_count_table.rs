//! The trace states every source file's line count (`meta.dat` bit 14,
//! `FLAG_HAS_LINE_COUNT_TABLE`), and the writer the recorder configures
//! refuses a step past a file's last line.
//!
//! Without the table a reader lays every file out at the writer's
//! 100 000-lines-per-file convention, which the container does not record.
//! With it, each `paths.dat` record carries the file's real size, and a step
//! one line past the end — whose address would fall inside the NEXT file's
//! range and read back as a location that was never recorded — is refused at
//! the only place that can see it: the writer.
//!
//! No mocks: the recorder runs on real Aiken fixtures, the container it writes
//! is read back through the pure-Rust `codetracer_trace_reader`, and the
//! refusal is observed on the production Nim writer.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use codetracer_cardano_recorder::line_counts::{
    line_counted_writer, source_line_count, LineCountedPaths,
};
use codetracer_trace_reader::interning_tables_reader::open_interning_tables;
use codetracer_trace_types::Line;
use codetracer_trace_writer_nim::trace_writer::TraceWriter;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-programs/aiken")
        .join(name)
}

fn ct_file(out_dir: &Path) -> PathBuf {
    std::fs::read_dir(out_dir)
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|ext| ext == "ct"))
        .unwrap_or_else(|| panic!("expected a .ct container in {out_dir:?}"))
}

/// `path → recorded line count` for every path in the container.
fn recorded_line_counts(ct: &Path) -> BTreeMap<String, u64> {
    let tables = open_interning_tables(ct)
        .expect("interning tables decode")
        .expect("paths.dat present");
    assert_eq!(
        tables.line_counts().len(),
        tables.path_count(),
        "every paths.dat record must state a line count; the container states {} for {} path(s)",
        tables.line_counts().len(),
        tables.path_count()
    );
    (0..tables.path_count() as u64)
        .map(|id| {
            (
                tables.path_str(id).expect("path decodes"),
                tables.line_count(id).expect("line count present"),
            )
        })
        .collect()
}

#[test]
fn the_trace_states_the_line_count_of_every_source_file() {
    // A fixture that imports a `lib/` module, so the table has to cover a
    // file the recorder discovers while tracing, not only the one it was given.
    let tmp = tempfile::tempdir().expect("tempdir");
    let out_dir = tmp.path().join("traces");
    let source = fixture("module_imports_test.ak");
    codetracer_cardano_recorder::recorder::record(&source, &out_dir)
        .expect("recorder::record should succeed");

    let counts = recorded_line_counts(&ct_file(&out_dir));
    assert!(
        counts.len() >= 2,
        "the fixture steps through the main file and its imported module; paths: {counts:?}"
    );
    for (path, count) in &counts {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("recorded path {path} must be a readable source: {e}"));
        let want = text.lines().count().max(1) as u64 + u64::from(text.ends_with('\n'));
        assert_eq!(
            *count, want,
            "{path} has {want} line(s) (newlines + 1); the trace records {count}"
        );
    }
    assert!(
        counts.contains_key(&source.to_string_lossy().into_owned()),
        "the main source must be one of the recorded paths: {counts:?}"
    );
}

#[test]
fn a_step_past_a_files_last_line_is_refused_by_the_writer() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let trace_path = tmp.path().join("trace.ctfs");
    let source = fixture("flow_test.ak");
    let text = std::fs::read_to_string(&source).expect("read fixture");
    let last = source_line_count(&text) as i64;

    let mut writer = line_counted_writer("overflow", &trace_path).expect("line-counted writer");
    let mut paths = LineCountedPaths::default();
    paths
        .register(&mut writer, &source, &text)
        .expect("register the fixture");
    // Registering again is a no-op, not a second record for the same file.
    paths
        .register(&mut writer, &source, &text)
        .expect("re-register the fixture");

    // The file's own last line is inside its slot, so the writer is not simply
    // refusing everything. `last_error` is sticky and per-thread, so this also
    // pins the known-empty state the assertion below reads against.
    TraceWriter::start(&mut writer, &source, Line(last));
    assert_eq!(
        codetracer_trace_writer_nim::last_error(),
        "",
        "the file's own last line must be accepted"
    );

    // One past it is not. The refusal surfaces when the buffered step flushes.
    TraceWriter::register_step(&mut writer, &source, Line(last + 1));
    TraceWriter::register_step(&mut writer, &source, Line(1));
    let err = codetracer_trace_writer_nim::last_error();
    assert!(
        err.contains(&*source.to_string_lossy()) && err.contains(&last.to_string()),
        "a step at line {} of a file recorded as having {last} line(s) must be refused by name; \
         last_error was: {err:?}",
        last + 1
    );
}

/// Every fixture records within its files' stated sizes. The writer refuses a
/// step past a file's last line, and the refusal fails the recording when the
/// trace is finished, so a recorder that stepped past a file's end would stop
/// producing traces for that program. Fixtures the recorder rejects for other
/// reasons are covered by the suites that exercise them.
#[test]
fn no_fixture_steps_past_the_end_of_a_file() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-programs/aiken");
    let mut fixtures: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "ak"))
        .collect();
    fixtures.sort();
    assert!(!fixtures.is_empty(), "no fixtures under {dir:?}");

    let mut recorded = 0;
    for source in fixtures {
        let tmp = tempfile::tempdir().expect("tempdir");
        let out_dir = tmp.path().join("traces");
        match codetracer_cardano_recorder::recorder::record(&source, &out_dir) {
            Ok(()) => recorded += 1,
            Err(e) => {
                let msg = format!("{e:#}");
                assert!(
                    !msg.contains("line(s)")
                        && !msg.contains("line-count")
                        && !msg.contains("register"),
                    "recording {} was refused by the line-count table: {msg}",
                    source.display()
                );
            }
        }
    }
    assert!(recorded > 0, "no fixture recorded at all");
}
