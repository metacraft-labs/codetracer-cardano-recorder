//! Recording logic for Aiken execution traces.
//!
//! This module provides the top-level `record` function that reads an Aiken
//! source file, evaluates variable assignments, captures the trace, and writes
//! CodeTracer output.

use std::path::Path;

use codetracer_trace_writer::TraceEventsFileFormat;
use eyre::{Context, Result};

use crate::tracer::AikenTracer;

/// Record an Aiken execution trace.
///
/// Reads the Aiken source file at `source_path`, parses function definitions
/// and variable assignments, evaluates them, captures the trace, and writes
/// CodeTracer trace files to `out_dir`.
pub fn record(source_path: &Path, out_dir: &Path, format: TraceEventsFileFormat) -> Result<()> {
    let source_code = std::fs::read_to_string(source_path)
        .with_context(|| format!("failed to read source file: {}", source_path.display()))?;

    AikenTracer::trace_program(source_path, &source_code, out_dir, format)
}
