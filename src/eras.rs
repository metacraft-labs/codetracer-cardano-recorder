//! Cardano's ledger eras, their cost models, and the two different floors
//! people mean by "floor: genesis".
//!
//! # What this module observes
//!
//! `/epoch_params`, per epoch: the era name, the protocol version, and the
//! per-language Plutus cost-model parameters in the chain's own index order.
//! From those it derives
//!
//! - **the era boundary series** — every epoch at which the announced protocol
//!   version changed, which is where script semantics and cost models change
//!   with it; and
//! - **a cost model per era**, built through
//!   `uplc::machine::cost_model::initialize_cost_model` from the chain's own
//!   parameters, so a transaction can be evaluated under the rules its own era
//!   charged rather than under today's.
//!
//! # The boundary is a property of the EPOCH, not of the block header
//!
//! `/blocks` carries `proto_major`/`proto_minor` per block, and it is tempting
//! to read the era off them. CXP-ADA-2 measured that this is wrong, in both
//! directions: blocks announcing `proto_major = 5` appear at height 6,167,631
//! in **epoch 286**, three epochs before Alonzo began, and blocks still
//! announcing `4` appear at height 6,235,699 in epoch 289. Those are block
//! producers advertising readiness, not the ledger switching. The era comes
//! from the epoch's parameters, and this module takes it only from there.
//!
//! # Two floors, and they are different numbers
//!
//! Chain-Support-Matrix.md §1.3 records Cardano's reach as *genesis*. That is
//! true of **acquisition** and false of **script tracing**, and conflating
//! them produces a "floor: genesis" that no Plutus transaction could ever
//! demonstrate:
//!
//! - the **acquisition floor** is genesis: blocks and transactions are served
//!   from epoch 0;
//! - the **script-tracing floor** is the first epoch that has a Plutus cost
//!   model at all, because before it there is no script to trace and no model
//!   to charge it with.
//!
//! [`floors`] measures both and reports both.
//!
//! # What this module does NOT do
//!
//! It does not evaluate, and it does not reach into `crate::cek`. Building a
//! cost model is as far as it goes; applying one to a settled transaction
//! needs a script context, and that is [`crate::oracle`]'s job — which calls
//! [`cost_model_for`] rather than the reverse, so the dependency runs one way
//! and this module stays unable to evaluate anything. It also refuses,
//! rather than pads, a cost model whose parameter vector is shorter than the
//! evaluator reads — a padded model would charge invented prices and report a
//! budget as though it were the chain's.

use std::collections::BTreeMap;
use std::fmt;

use eyre::{eyre, Result};
use pallas_primitives::conway::Language;
use uplc::machine::cost_model::{initialize_cost_model, CostModel};

use crate::chain::{ChainSource, EpochParams, ProtocolVersionRow};

/// How many positional parameters the vendored UPLC evaluator reads for each
/// Plutus language.
///
/// These are counted from `initialize_cost_model`'s own `costs[N]` indices in
/// `metacraft-aiken-fork/crates/uplc`, not guessed: V1 reads 0..=165, V2
/// 0..=174 and V3 0..=296. A chain parameter vector shorter than the figure
/// here cannot produce that era's prices, and one longer carries parameters
/// this evaluator does not read.
pub const EVALUATOR_ARITY: [(&str, usize); 3] =
    [("PlutusV1", 166), ("PlutusV2", 175), ("PlutusV3", 297)];

/// The evaluator's arity for a language name, if it is one.
pub fn evaluator_arity(language: &str) -> Option<usize> {
    EVALUATOR_ARITY
        .iter()
        .find(|(l, _)| *l == language)
        .map(|(_, n)| *n)
}

/// Map a Koios cost-model language name onto the evaluator's `Language`.
pub fn language_of(name: &str) -> Option<Language> {
    match name {
        "PlutusV1" => Some(Language::PlutusV1),
        "PlutusV2" => Some(Language::PlutusV2),
        "PlutusV3" => Some(Language::PlutusV3),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Boundaries
// ---------------------------------------------------------------------------

/// An epoch at which the announced protocol version changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Boundary {
    pub first_epoch: u64,
    pub protocol_major: u64,
    pub protocol_minor: u64,
}

/// Every epoch at which the protocol version changed, ascending.
///
/// The first epoch that has a parameter row at all counts as a boundary: it is
/// where the chain starts having a protocol version, and below it is Byron.
pub fn boundaries(rows: &[ProtocolVersionRow]) -> Vec<Boundary> {
    let mut out: Vec<Boundary> = Vec::new();
    for r in rows {
        let changed = match out.last() {
            None => true,
            Some(prev) => {
                prev.protocol_major != r.protocol_major || prev.protocol_minor != r.protocol_minor
            }
        };
        if changed {
            out.push(Boundary {
                first_epoch: r.epoch_no,
                protocol_major: r.protocol_major,
                protocol_minor: r.protocol_minor,
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The era table
// ---------------------------------------------------------------------------

/// Whether this evaluator can be given an era's own prices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArityVerdict {
    /// The era's parameter vector is exactly what the evaluator reads.
    Exact,
    /// Longer than the evaluator reads: the prefix is used and the surplus is
    /// reported, because a parameter this evaluator ignores is a price the
    /// chain charged and the replay will not.
    ///
    /// **Measured 2026-10-31: on this chain's current epoch the surplus does
    /// not in fact move the budget.** Epoch 664 serves 350 PlutusV3
    /// parameters against the 297 this evaluator reads — `Surplus(53)` — and
    /// four mainnet PlutusV3 redeemers, across the two PlutusV3 transactions
    /// the corpus carries, replayed from that epoch's
    /// 297-parameter prefix reproduced the execution units the chain recorded
    /// **exactly** (`tests/test_budget_oracle.rs`). The surplus parameters are
    /// prices for builtins those scripts do not use, so they were never
    /// charged. That is a reading about those transactions and not a
    /// guarantee: a script that used one of the 53 would be mispriced, and the
    /// surplus is still reported for exactly that reason.
    Surplus(usize),
    /// Shorter than the evaluator reads: this era's prices cannot be supplied,
    /// so a replay under them is refused rather than approximated.
    Short(usize),
    /// The era has no cost model for this language at all.
    Absent,
}

impl fmt::Display for ArityVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArityVerdict::Exact => write!(f, "exact"),
            ArityVerdict::Surplus(n) => write!(f, "surplus+{n}"),
            ArityVerdict::Short(n) => write!(f, "SHORT-{n}"),
            ArityVerdict::Absent => write!(f, "absent"),
        }
    }
}

/// One era row: an epoch, its era, and what it can charge.
#[derive(Debug, Clone)]
pub struct EraRow {
    pub epoch_no: u64,
    /// `None` for an epoch with no parameter row — every Byron epoch.
    pub era: Option<String>,
    pub protocol_major: Option<u64>,
    pub protocol_minor: Option<u64>,
    /// Per-language parameter counts the chain served for this epoch.
    pub cost_model_sizes: BTreeMap<String, usize>,
    /// Per-language arity verdict against the evaluator.
    pub arity: BTreeMap<String, ArityVerdict>,
}

impl EraRow {
    /// Whether this epoch has any Plutus cost model at all, which is the same
    /// question as whether a script could execute in it.
    pub fn can_execute_scripts(&self) -> bool {
        !self.cost_model_sizes.is_empty()
    }
}

/// Read the era table for the given epochs.
pub fn era_table(src: &dyn ChainSource, epochs: &[u64]) -> Result<Vec<EraRow>> {
    let mut out = Vec::new();
    for &e in epochs {
        out.push(era_row(&src.epoch_params(e)?));
    }
    Ok(out)
}

/// Turn one epoch's parameters into an era row.
pub fn era_row(p: &EpochParams) -> EraRow {
    let mut sizes = BTreeMap::new();
    let mut arity = BTreeMap::new();
    for (lang, params) in &p.cost_models {
        sizes.insert(lang.clone(), params.len());
        let verdict = match evaluator_arity(lang) {
            None => ArityVerdict::Absent,
            Some(want) => match params.len().cmp(&want) {
                std::cmp::Ordering::Equal => ArityVerdict::Exact,
                std::cmp::Ordering::Greater => ArityVerdict::Surplus(params.len() - want),
                std::cmp::Ordering::Less => ArityVerdict::Short(want - params.len()),
            },
        };
        arity.insert(lang.clone(), verdict);
    }
    // A language the era does not carry is `Absent` rather than missing from
    // the map, so a report shows the gap instead of leaving a reader to infer
    // it from a shorter row.
    for (lang, _) in EVALUATOR_ARITY {
        arity
            .entry(lang.to_string())
            .or_insert(ArityVerdict::Absent);
    }
    EraRow {
        epoch_no: p.epoch_no,
        era: p.era.clone(),
        protocol_major: p.protocol_major,
        protocol_minor: p.protocol_minor,
        cost_model_sizes: sizes,
        arity,
    }
}

// ---------------------------------------------------------------------------
// Cost models
// ---------------------------------------------------------------------------

/// Build the cost model one era charged for one Plutus language.
///
/// Refuses when the era carries no model for that language, and refuses when
/// the model is shorter than the evaluator reads. The second refusal is the
/// load-bearing one: `initialize_cost_model` indexes its slice positionally,
/// so a short vector would either panic or — worse, had it been padded — price
/// builtins at numbers no epoch ever set.
pub fn cost_model_for(p: &EpochParams, language: &str) -> Result<CostModel> {
    let params = p.cost_models.get(language).ok_or_else(|| {
        // Two different absences, and conflating them would misreport the
        // floor. An epoch with NO cost model at all is below the
        // script-tracing floor: no Plutus script could run in it. An epoch
        // that has other languages but not this one is above the floor and
        // simply predates this language.
        let why = if p.cost_models.is_empty() {
            "that epoch carries no Plutus cost model for any language, so no Plutus script \
             could execute in it at all — it is below the script-tracing floor"
                .to_string()
        } else {
            format!(
                "that epoch does carry cost model(s) for {}, so it is above the script-tracing \
                 floor and simply predates {language}",
                p.cost_models.keys().cloned().collect::<Vec<_>>().join("+")
            )
        };
        eyre!(
            "epoch {} ({}) carries no {language} cost model: {why}",
            p.epoch_no,
            p.era.as_deref().unwrap_or("pre-Shelley / no parameter row")
        )
    })?;
    let want = evaluator_arity(language)
        .ok_or_else(|| eyre!("{language} is not a Plutus language this evaluator knows"))?;
    if params.len() < want {
        return Err(eyre!(
            "epoch {} ({}) serves {} {language} parameter(s) and this evaluator reads {want}; \
             {} are missing. Padding them would charge prices the chain never set, so the era's \
             own rules are unavailable and the replay is refused rather than approximated",
            p.epoch_no,
            p.era.as_deref().unwrap_or("unknown era"),
            params.len(),
            want - params.len()
        ));
    }
    let lang = language_of(language)
        .ok_or_else(|| eyre!("{language} has no `Language` in the evaluator"))?;
    Ok(initialize_cost_model(&lang, &params[..want]))
}

// ---------------------------------------------------------------------------
// Floors
// ---------------------------------------------------------------------------

/// The two floors, measured rather than quoted.
#[derive(Debug, Clone)]
pub struct Floors {
    /// Lowest epoch for which a block was enumerated. Genesis is epoch 0.
    pub acquisition_floor_epoch: u64,
    /// The epoch the acquisition floor was demonstrated at, and how.
    pub acquisition_evidence: String,
    /// First epoch carrying any Plutus cost model, or `None` if none of the
    /// epochs read carries one.
    pub script_tracing_floor_epoch: Option<u64>,
    pub script_tracing_era: Option<String>,
    pub script_tracing_languages: Vec<String>,
    /// The epochs that were read, so the reading's resolution is visible: a
    /// floor derived from a sampled epoch list is a floor to within that list.
    pub epochs_read: Vec<u64>,
}

/// Derive both floors from an era table.
///
/// `acquisition_floor_epoch` is 0 when the table carries epoch 0 — the point
/// being that the epoch is *reachable*, which for Cardano it is, and that it
/// carries no protocol-parameter row, which is why it cannot be a
/// script-tracing floor.
pub fn floors(rows: &[EraRow]) -> Floors {
    let epochs_read: Vec<u64> = rows.iter().map(|r| r.epoch_no).collect();
    let acquisition = rows.iter().map(|r| r.epoch_no).min().unwrap_or(0);
    let first_plutus = rows
        .iter()
        .filter(|r| r.can_execute_scripts())
        .min_by_key(|r| r.epoch_no);
    Floors {
        acquisition_floor_epoch: acquisition,
        acquisition_evidence: match rows.iter().find(|r| r.epoch_no == acquisition) {
            Some(r) if r.era.is_none() => format!(
                "epoch {acquisition} answers /epoch_params with an EMPTY row set — it is a Byron \
                 epoch with no protocol-parameter row at all, which is the strongest available \
                 statement that acquisition reaches below every era that has parameters"
            ),
            Some(r) => format!(
                "epoch {acquisition} answers /epoch_params as era {}",
                r.era.as_deref().unwrap_or("?")
            ),
            None => "no epoch was read".to_string(),
        },
        script_tracing_floor_epoch: first_plutus.map(|r| r.epoch_no),
        script_tracing_era: first_plutus.and_then(|r| r.era.clone()),
        script_tracing_languages: first_plutus
            .map(|r| r.cost_model_sizes.keys().cloned().collect())
            .unwrap_or_default(),
        epochs_read,
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render the era table and the floors.
pub fn render(rows: &[EraRow], boundaries: &[Boundary], f: &Floors) -> String {
    let mut s = String::new();

    s.push_str("protocol-version boundaries (from /epoch_params, NOT from block headers):\n");
    for b in boundaries {
        s.push_str(&format!(
            "  first epoch {:>4}  protocol {}.{}\n",
            b.first_epoch, b.protocol_major, b.protocol_minor
        ));
    }
    s.push_str(&format!(
        "  -- {} boundary/boundaries\n\n",
        boundaries.len()
    ));

    s.push_str("era table (one row per epoch read):\n");
    s.push_str(&format!(
        "  {:>5}  {:<9} {:<6} {:<34} {}\n",
        "epoch", "era", "proto", "cost-model sizes", "arity vs this evaluator"
    ));
    for r in rows {
        let sizes = if r.cost_model_sizes.is_empty() {
            "none".to_string()
        } else {
            r.cost_model_sizes
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let arity = r
            .arity
            .iter()
            .filter(|(_, v)| **v != ArityVerdict::Absent)
            .map(|(k, v)| format!("{k}:{v}"))
            .collect::<Vec<_>>()
            .join(" ");
        s.push_str(&format!(
            "  {:>5}  {:<9} {:<6} {:<34} {}\n",
            r.epoch_no,
            r.era.as_deref().unwrap_or("-"),
            match (r.protocol_major, r.protocol_minor) {
                (Some(a), Some(b)) => format!("{a}.{b}"),
                _ => "-".to_string(),
            },
            sizes,
            if arity.is_empty() {
                "-".to_string()
            } else {
                arity
            }
        ));
    }

    s.push_str("\nfloors, measured separately because they are different numbers:\n");
    s.push_str(&format!(
        "  acquisition floor     epoch {}\n    {}\n",
        f.acquisition_floor_epoch, f.acquisition_evidence
    ));
    match f.script_tracing_floor_epoch {
        Some(e) => s.push_str(&format!(
            "  script-tracing floor  epoch {} (era {}, cost model(s) {})\n    the first epoch \
             read that carries any Plutus cost model; below it a transaction cannot contain a \
             script execution, so \"tracing a transaction from early history\" is not satisfiable \
             below this epoch however good the acquisition is\n",
            e,
            f.script_tracing_era.as_deref().unwrap_or("?"),
            f.script_tracing_languages.join("+")
        )),
        None => s.push_str(
            "  script-tracing floor  NOT MEASURED — none of the epochs read carries a cost model\n",
        ),
    }
    s.push_str(&format!(
        "  resolution: {} epoch(s) read ({:?}), so each floor is exact to within that list\n",
        f.epochs_read.len(),
        f.epochs_read
    ));
    s
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn row(epoch: u64, major: u64, minor: u64) -> ProtocolVersionRow {
        ProtocolVersionRow {
            epoch_no: epoch,
            protocol_major: major,
            protocol_minor: minor,
        }
    }

    #[test]
    fn boundaries_fire_on_a_minor_bump_too() {
        // Cardano's intra-era hard forks move the MINOR version (Alonzo 5.0 ->
        // 6.0 is major, but a minor bump changes parameters too), so a
        // boundary detector keyed on major alone would miss them.
        let rows = [row(1, 5, 0), row(2, 5, 0), row(3, 5, 1), row(4, 6, 0)];
        let b = boundaries(&rows);
        assert_eq!(b.len(), 3);
        assert_eq!(b[1].first_epoch, 3);
        assert_eq!((b[1].protocol_major, b[1].protocol_minor), (5, 1));
    }

    #[test]
    fn a_short_cost_model_is_refused_rather_than_padded() {
        let p = EpochParams {
            epoch_no: 507,
            era: Some("Conway".into()),
            protocol_major: Some(9),
            protocol_minor: Some(0),
            cost_models: BTreeMap::from([("PlutusV3".to_string(), vec![1i64; 251])]),
        };
        let err = cost_model_for(&p, "PlutusV3").expect_err("251 < 297 must be refused");
        let msg = err.to_string();
        assert!(
            msg.contains("251") && msg.contains("297") && msg.contains("46 are missing"),
            "the refusal must state both arities and the shortfall; got: {msg}"
        );
    }

    #[test]
    fn a_pre_alonzo_epoch_has_no_model_to_charge_with() {
        let p = EpochParams {
            epoch_no: 251,
            era: Some("Mary".into()),
            protocol_major: Some(4),
            protocol_minor: Some(0),
            cost_models: BTreeMap::new(),
        };
        let err = cost_model_for(&p, "PlutusV1").expect_err("Mary has no Plutus model");
        assert!(
            err.to_string().contains("below the script-tracing floor"),
            "an epoch with no cost model at all must be reported as below the floor; got: {err}"
        );
    }

    #[test]
    fn a_language_a_later_era_added_is_not_reported_as_below_the_floor() {
        // Alonzo has PlutusV1 and no PlutusV2. That is not the same absence as
        // Mary having none at all, and reporting it the same way would put the
        // script-tracing floor in the wrong era.
        let p = EpochParams {
            epoch_no: 290,
            era: Some("Alonzo".into()),
            protocol_major: Some(5),
            protocol_minor: Some(0),
            cost_models: BTreeMap::from([("PlutusV1".to_string(), vec![7i64; 166])]),
        };
        let err = cost_model_for(&p, "PlutusV2").expect_err("Alonzo has no PlutusV2");
        let msg = err.to_string();
        assert!(
            msg.contains("above the script-tracing floor") && msg.contains("predates PlutusV2"),
            "got: {msg}"
        );
    }

    #[test]
    fn an_exact_model_builds() {
        let p = EpochParams {
            epoch_no: 290,
            era: Some("Alonzo".into()),
            protocol_major: Some(5),
            protocol_minor: Some(0),
            cost_models: BTreeMap::from([("PlutusV1".to_string(), vec![7i64; 166])]),
        };
        cost_model_for(&p, "PlutusV1").expect("166 parameters is exactly what V1 reads");
        assert_eq!(era_row(&p).arity["PlutusV1"], ArityVerdict::Exact);
    }

    #[test]
    fn surplus_parameters_are_reported_not_hidden() {
        let p = EpochParams {
            epoch_no: 644,
            era: Some("Conway".into()),
            protocol_major: Some(11),
            protocol_minor: Some(0),
            cost_models: BTreeMap::from([("PlutusV1".to_string(), vec![7i64; 332])]),
        };
        assert_eq!(era_row(&p).arity["PlutusV1"], ArityVerdict::Surplus(166));
        cost_model_for(&p, "PlutusV1").expect("a surplus is usable as a prefix");
    }

    #[test]
    fn the_two_floors_are_not_the_same_number() {
        let byron = EraRow {
            epoch_no: 0,
            era: None,
            protocol_major: None,
            protocol_minor: None,
            cost_model_sizes: BTreeMap::new(),
            arity: BTreeMap::new(),
        };
        let mary = EraRow {
            epoch_no: 251,
            era: Some("Mary".into()),
            protocol_major: Some(4),
            protocol_minor: Some(0),
            cost_model_sizes: BTreeMap::new(),
            arity: BTreeMap::new(),
        };
        let alonzo = EraRow {
            epoch_no: 290,
            era: Some("Alonzo".into()),
            protocol_major: Some(5),
            protocol_minor: Some(0),
            cost_model_sizes: BTreeMap::from([("PlutusV1".to_string(), 166)]),
            arity: BTreeMap::new(),
        };
        let f = floors(&[byron, mary, alonzo]);
        assert_eq!(f.acquisition_floor_epoch, 0);
        assert_eq!(f.script_tracing_floor_epoch, Some(290));
        assert_ne!(
            Some(f.acquisition_floor_epoch),
            f.script_tracing_floor_epoch,
            "if these were ever equal the whole distinction would be decoration"
        );
    }
}
