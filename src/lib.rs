//! CodeTracer recorder for Aiken/Cardano smart contract programs.
//!
//! Two paths live here and they are graded differently.
//!
//! **Script tracing** ([`tracer`], [`recorder`]) parses Aiken source and walks
//! its own AST. Chain-Support-Matrix.md §2.1 grades that *source-simulated*
//! and caps it at fidelity C; §2.2 records *"compiler never invoked"*.
//! Replacing it is CXP-ADA-1's deliverables 2 and 3 and is blocked on the
//! Aiken compiler front-end. [`cek`] is the authoritative replacement path:
//! it steps the chain's own UPLC CEK machine.
//!
//! **Acquisition** ([`chain`], [`koios`], [`fixture`], [`enumerate`],
//! [`provenance`], [`eras`], [`prestate`]) is CXP-ADA-2. It reads blocks,
//! transactions and the state leaves a script execution needs, and it reports
//! per requisite whether the transaction carries it or a node has to answer
//! for it. Its prestate strategy is `hydrated-from-node`
//! ([`prestate::PRESTATE_STRATEGY`]), measured rather than asserted.
//!
//! [`blockfrost`] predates the acquisition path and needs an API key no lane
//! in this workspace has; [`koios`] is the keyless route and is what the
//! acquisition path uses.

pub mod blockfrost;
pub mod cek;
pub mod chain;
pub mod enumerate;
pub mod eras;
pub mod fixture;
pub mod koios;
pub mod prestate;
pub mod provenance;
pub mod recorder;
pub mod source_map;
pub mod tracer;
pub mod transaction;
