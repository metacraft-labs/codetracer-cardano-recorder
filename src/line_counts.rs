//! Per-file line counts in the trace's `paths.dat` (`meta.dat` bit 14,
//! `FLAG_HAS_LINE_COUNT_TABLE`).
//!
//! The recorder has the text of every source file it steps through, so it
//! states each file's real size instead of leaving readers to lay files out
//! at the writer's 100 000-lines-per-file convention. With the sizes stated,
//! the writer also refuses a step past a file's last line: such a step's
//! address would fall inside the next file's range and read back as a
//! location that was never recorded.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use codetracer_trace_writer_nim::trace_writer::TraceWriter;
use codetracer_trace_writer_nim::{NimTraceWriter, TraceEventsFileFormat};
use eyre::{eyre, Result};

/// The number of lines `text` occupies: its newline count plus one, so the
/// final line counts whether or not the text ends in a newline.
pub fn source_line_count(text: &str) -> u64 {
    text.bytes().filter(|&b| b == b'\n').count() as u64 + 1
}

/// A CTFS writer, already writing its events to `events_path`, that records a
/// line count for every path it registers.
///
/// The writer only accepts the table once its event streams are open, which
/// is why this opens them too. Every path a step or function names must then
/// be registered through [`LineCountedPaths::register`] before it is used:
/// with the table on, the writer refuses the implicit registration a step
/// would otherwise perform.
pub fn line_counted_writer(program: &str, events_path: &Path) -> Result<NimTraceWriter> {
    let mut writer = NimTraceWriter::new(program, &[], TraceEventsFileFormat::Ctfs);
    TraceWriter::begin_writing_trace_events(&mut writer, events_path)
        .map_err(|e| eyre!("cannot open {}: {e}", events_path.display()))?;
    writer
        .enable_line_count_table()
        .map_err(|e| eyre!("cannot enable the line-count table: {e}"))?;
    Ok(writer)
}

/// The paths already registered on a line-counted writer. A path is
/// registered once; registering it again would record a second version.
#[derive(Debug, Default)]
pub struct LineCountedPaths {
    registered: HashSet<PathBuf>,
}

impl LineCountedPaths {
    /// Register `path` with the line count of `text`, unless it already is.
    pub fn register(&mut self, writer: &mut NimTraceWriter, path: &Path, text: &str) -> Result<()> {
        if self.registered.contains(path) {
            return Ok(());
        }
        writer
            .register_path_with_line_count(path, source_line_count(text))
            .map_err(|e| eyre!("cannot register {}: {e}", path.display()))?;
        self.registered.insert(path.to_path_buf());
        Ok(())
    }
}
