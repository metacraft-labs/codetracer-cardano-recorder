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
//! **The budget oracle** ([`oracle`]) is CXP-ADA-1 deliverable 4 and is the
//! strongest check in this crate. It reconstructs Plutus's `ScriptContext` for
//! a settled transaction, steps every script through [`cek`] under the cost
//! model of the epoch the transaction settled in ([`eras`]), and compares each
//! redeemer's budget against the execution units the transaction's own witness
//! set declares — a figure the network produced, so no bug here can move it.
//! What "agreement" means was measured and is not equality in general: the
//! ledger's rule is that execution *fits inside* the declaration, and over 463
//! live mainnet transactions 58 of 64 redeemers were exact and 6 were under.
//! See the module docs.
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
//! acquisition path and the budget oracle use. `blockfrost` no longer offers
//! an executable `TransactionData`: it cannot read a redeemer's data (its
//! endpoint serves the data hash) or assemble a script context, and it now
//! reports what it read instead of returning a placeholder.

pub mod blockfrost;
pub mod cek;
pub mod chain;
pub mod enumerate;
pub mod eras;
pub mod fixture;
pub mod koios;
pub mod oracle;
pub mod prestate;
pub mod provenance;
pub mod recorder;
pub mod source_map;
pub mod tracer;
pub mod transaction;
