//! CodeTracer recorder for Aiken/Cardano smart contract programs.
//!
//! This crate captures execution traces from Aiken programs by parsing
//! function definitions, variable declarations, and expressions,
//! evaluating them in order, and converting the results into the
//! CodeTracer trace format for debugging and analysis.

pub mod recorder;
pub mod source_map;
pub mod tracer;
