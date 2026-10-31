//! The prestate strategy this recorder declares, and the re-measurement that
//! retired the one it used to declare.
//!
//! # What this module observes
//!
//! Nothing directly. It takes [`crate::provenance::TxProvenance`] readings —
//! which are hashes compared against the chain's own bytes — and turns them
//! into one verdict: whether the `self-contained` claim holds, and therefore
//! which of Chain-Support-Matrix.md §1.4's **seven closed-set tokens** this
//! producer may write into `provenance.prestateStrategy`.
//!
//! # The declaration
//!
//! [`PRESTATE_STRATEGY`] is `hydrated-from-node`. §1.4 defines that row as a
//! narrowing of `prestate-trace` *"for a chain that serves no single minimal
//! pre-state method. The execution runs, and every time it reads state the
//! node is asked for that leaf alone at a stated pre-state block, in as many
//! rounds as the execution takes."* That is exactly the shape of
//! [`crate::chain::ChainSource`]: batched leaf reads by output reference, with
//! the round count reported.
//!
//! [`RETIRED_STRATEGY`] is `self-contained`, which §3.11 asserted for Cardano
//! and which this module exists to **re-measure rather than cite**. The
//! charter's 2026-09-28 probe is prose in a milestone file; a prose citation
//! cannot go red when the chain changes, so the reading is reproduced here as
//! code over committed captures and the controls run every time.
//!
//! # Why the token changed, in one sentence per arm
//!
//! - **Negative arm (post-Babbage):** on a live PlutusV3 spend, no datum in
//!   the witness set hashes to the spent output's `datum_hash`, and for most
//!   executions no script in the witness set hashes to the script hash the
//!   spent address names — CIP-32 left the datum at the output and CIP-33 left
//!   the script at another one.
//! - **Positive arm (Alonzo):** on the earliest Plutus execution mainnet has,
//!   the script's 2,739 bytes *are* in the witness set and the spent output's
//!   datum *is* too. So `self-contained` was true of the era §3.11 was written
//!   against. The token did not change because the measurement improved; the
//!   protocol changed underneath it.
//! - **Instrument control:** both arms are read with the same two-sided search
//!   ([`crate::provenance::SearchControls`]), so an `ABSENT` is attributable.
//!
//! # What this module does NOT do
//!
//! It does not emit a snapshot tree and it does not write
//! `provenance.prestateStrategy` anywhere. Emitting a
//! `blocktracer/chain-snapshot@1` tree is **CXP-ADA-3**, and Data-Contract.md
//! §5's producer filesystem contract is that milestone's to satisfy. What is
//! here is the token and the evidence for it, so that CXP-ADA-3 writes a
//! declared value rather than choosing one.

use std::fmt;

use crate::provenance::{Carriage, TxProvenance};

/// The token this producer declares, verbatim from §1.4's closed set.
pub const PRESTATE_STRATEGY: &str = "hydrated-from-node";

/// The token §3.11 asserted for Cardano, retired by measurement.
pub const RETIRED_STRATEGY: &str = "self-contained";

/// Every token §1.4's closed set admits, so a declaration can be checked
/// against the set rather than spell-checked by eye.
///
/// Transcribed from Chain-Support-Matrix.md §1.4. A token this producer might
/// want that is not here is **a gap in that section**, and the remedy §1.4
/// states is a row there — never a new string in a snapshot.
pub const CLOSED_SET: [&str; 7] = [
    "version-addressed",
    "self-contained",
    "prestate-trace",
    "replay-preceding",
    "hydrated-from-node",
    "self-contained-circuit",
    "forward-capture",
];

/// Whether the declared token is in the closed set.
pub fn declaration_is_in_closed_set() -> bool {
    CLOSED_SET.contains(&PRESTATE_STRATEGY)
}

// ---------------------------------------------------------------------------
// The verdict
// ---------------------------------------------------------------------------

/// What a set of transaction readings says about self-containment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfContainmentVerdict {
    /// Every requisite of every execution read travels with its transaction.
    Holds,
    /// At least one requisite requires a state read.
    Refuted,
    /// No execution was read, so nothing was decided. Not a pass.
    NotMeasured,
}

impl fmt::Display for SelfContainmentVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SelfContainmentVerdict::Holds => write!(f, "HOLDS"),
            SelfContainmentVerdict::Refuted => write!(f, "REFUTED"),
            SelfContainmentVerdict::NotMeasured => write!(f, "NOT MEASURED"),
        }
    }
}

/// One case's contribution to the reading.
#[derive(Debug, Clone)]
pub struct CaseReading {
    pub case: String,
    pub tx_hash: String,
    /// The ledger era, from the epoch of the block the transaction settled in,
    /// or `None` when it was not resolved. NOT the decoder probe.
    pub ledger_era: Option<String>,
    /// The newest format whose decoder accepted the bytes.
    pub decoded_as: String,
    pub executions: usize,
    /// Requisites that travel with the transaction, by name.
    pub in_transaction: Vec<String>,
    /// Requisites that need a state read, by name.
    pub requires_hydration: Vec<String>,
    pub self_contained: bool,
    pub two_sided_control: bool,
    pub body_hash_agrees: bool,
    pub hydration_rounds: u32,
}

/// The whole reading across every case.
#[derive(Debug, Clone)]
pub struct StrategyReading {
    pub declared: &'static str,
    pub retired: &'static str,
    pub declared_in_closed_set: bool,
    pub cases: Vec<CaseReading>,
    pub verdict: SelfContainmentVerdict,
    /// Cases in which every requisite travelled with the transaction.
    pub self_contained_cases: usize,
    /// Cases in which at least one did not.
    pub hydrated_cases: usize,
    /// Cases whose two-sided search control held.
    pub controlled_cases: usize,
}

impl StrategyReading {
    /// Whether the reading licenses the declared token.
    ///
    /// It does when self-containment is REFUTED — because that is what makes
    /// `hydrated-from-node` the right row rather than `self-contained` — and
    /// when every case that contributed had a working two-sided control. A
    /// refutation measured with a blind instrument is not a refutation.
    pub fn licenses_declaration(&self) -> bool {
        self.verdict == SelfContainmentVerdict::Refuted
            && self.controlled_cases == self.cases.len()
            && !self.cases.is_empty()
            && self.cases.iter().all(|c| c.body_hash_agrees)
    }
}

/// Read the strategy verdict from a set of named transaction provenances.
pub fn read(cases: &[(String, TxProvenance)]) -> StrategyReading {
    let mut readings = Vec::new();
    for (name, p) in cases {
        let mut in_tx: Vec<String> = Vec::new();
        let mut hydrated: Vec<String> = Vec::new();
        for e in &p.executions {
            for r in &e.requisites {
                let label = format!("{}[{}].{}", e.purpose, e.index, r.requisite);
                match r.carriage {
                    Carriage::InTransaction => in_tx.push(label),
                    Carriage::RequiresHydration => hydrated.push(label),
                    Carriage::NotApplicable => {}
                }
            }
        }
        readings.push(CaseReading {
            case: name.clone(),
            tx_hash: p.tx_hash.clone(),
            ledger_era: p.ledger_era.clone(),
            decoded_as: p.decoded_as.clone(),
            executions: p.executions.len(),
            self_contained: hydrated.is_empty() && !p.executions.is_empty(),
            two_sided_control: p.controls.two_sided(),
            body_hash_agrees: p.body_hash_agrees,
            hydration_rounds: p.hydration_rounds,
            in_transaction: in_tx,
            requires_hydration: hydrated,
        });
    }

    let any_execution = readings.iter().any(|c| c.executions > 0);
    let verdict = if !any_execution {
        SelfContainmentVerdict::NotMeasured
    } else if readings.iter().any(|c| !c.requires_hydration.is_empty()) {
        SelfContainmentVerdict::Refuted
    } else {
        SelfContainmentVerdict::Holds
    };

    StrategyReading {
        declared: PRESTATE_STRATEGY,
        retired: RETIRED_STRATEGY,
        declared_in_closed_set: declaration_is_in_closed_set(),
        self_contained_cases: readings.iter().filter(|c| c.self_contained).count(),
        hydrated_cases: readings
            .iter()
            .filter(|c| !c.requires_hydration.is_empty())
            .count(),
        controlled_cases: readings.iter().filter(|c| c.two_sided_control).count(),
        cases: readings,
        verdict,
    }
}

/// Render the strategy reading.
pub fn render(r: &StrategyReading) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "prestate strategy declared: {}\n  in Chain-Support-Matrix.md §1.4's closed set: {}\n",
        r.declared,
        if r.declared_in_closed_set {
            "yes"
        } else {
            "NO — THIS IS A GAP IN §1.4"
        }
    ));
    s.push_str(&format!(
        "retired claim re-measured: {} -> {}\n\n",
        r.retired, r.verdict
    ));

    for c in &r.cases {
        s.push_str(&format!(
            "  case {}\n    tx {} (ledger era {}, decoded as {})\n    {} execution(s); {} requisite(s) in-transaction, \
             {} requiring hydration; {} hydration round(s)\n",
            c.case,
            c.tx_hash,
            c.ledger_era.as_deref().unwrap_or("UNKNOWN"),
            c.decoded_as,
            c.executions,
            c.in_transaction.len(),
            c.requires_hydration.len(),
            c.hydration_rounds
        ));
        s.push_str(&format!(
            "    self-contained: {}   two-sided control: {}   body-hash identity: {}\n",
            if c.self_contained { "YES" } else { "no" },
            if c.two_sided_control {
                "held"
            } else {
                "FAILED"
            },
            if c.body_hash_agrees {
                "agrees"
            } else {
                "DISAGREES"
            }
        ));
        if !c.in_transaction.is_empty() {
            s.push_str(&format!(
                "      in-transaction: {}\n",
                c.in_transaction.join(", ")
            ));
        }
        if !c.requires_hydration.is_empty() {
            s.push_str(&format!(
                "      requires-hydration: {}\n",
                c.requires_hydration.join(", ")
            ));
        }
    }

    s.push_str(&format!(
        "\n  {} of {} case(s) self-contained; {} needing hydration; {} with a working two-sided \
         control\n",
        r.self_contained_cases,
        r.cases.len(),
        r.hydrated_cases,
        r.controlled_cases
    ));
    s.push_str(&format!(
        "  => the declaration `{}` is licensed by this reading: {}\n",
        r.declared,
        if r.licenses_declaration() {
            "YES"
        } else {
            "NO"
        }
    ));
    if r.verdict == SelfContainmentVerdict::Refuted && r.self_contained_cases > 0 {
        s.push_str(
            "  note: both arms are present — at least one case IS self-contained and at least \
             one is not, so this is an era regression rather than a blanket absence\n",
        );
    }
    s
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_declared_token_is_in_the_closed_set() {
        assert!(
            declaration_is_in_closed_set(),
            "a producer emitting a token outside §1.4's set is refused S5-PRESTATE-CLOSED with \
             zero objects written"
        );
    }

    #[test]
    fn the_retired_token_is_also_in_the_set_and_is_a_different_row() {
        // Retiring a token for this chain is not the same as deleting it from
        // the set: `self-contained` remains a §1.4 row that other chains use.
        assert!(CLOSED_SET.contains(&RETIRED_STRATEGY));
        assert_ne!(PRESTATE_STRATEGY, RETIRED_STRATEGY);
    }

    #[test]
    fn no_cases_is_not_measured_rather_than_holds() {
        let r = read(&[]);
        assert_eq!(r.verdict, SelfContainmentVerdict::NotMeasured);
        assert!(
            !r.licenses_declaration(),
            "an empty reading must not license a declaration"
        );
    }

    #[test]
    fn the_closed_set_is_seven_rows() {
        // §1.4 says "Seven strategies, and this table is a closed set". If
        // this count moves, the set here has drifted from the spec and the
        // declaration is being checked against the wrong vocabulary.
        assert_eq!(CLOSED_SET.len(), 7);
    }
}
