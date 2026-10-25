//! Stepping-driven observation of UPLC execution inside the chain's own CEK
//! machine.
//!
//! This module exists to satisfy CXP-ADA-1: Cardano script execution must be
//! observed *in the chain's own evaluator*, stepping through UPLC, rather than
//! by walking an AST the recorder parsed itself.  §2.1 of the charter is
//! explicit that a source-simulated recorder "must never be presented to a
//! user as an authoritative account of what the chain executed", so everything
//! here drives the real `uplc` CEK machine and reports what that machine did.
//!
//! The four per-step facets the milestone asks for come straight off the
//! machine, with no reconstruction on our side:
//!
//! | facet          | source                                              |
//! |----------------|-----------------------------------------------------|
//! | machine state  | the `MachineState` variant (`Compute`/`Return`/`Done`) |
//! | environment    | the `Env` carried by `MachineState::Compute`        |
//! | continuation   | the `Context` frame stack                           |
//! | budget         | `Machine::ex_budget` read at that step              |
//!
//! # Budget fidelity
//!
//! `step_program` constructs the machine exactly as `Program::eval_version`
//! does — `CostModel::default()` and slippage `200` — because the budget this
//! produces is compared against the execution units the chain recorded for the
//! transaction.  Any divergence in machine construction would make that
//! comparison meaningless, so the constants are deliberately not configurable.
//!
//! Driving `step()` to `Done` yields a final budget that is bit-identical to
//! `run()`'s: the machine flushes its batched machine-step costs inside
//! `return_compute`'s `Context::NoFrame` arm, i.e. on the very transition that
//! produces `Done`, so a stepped run settles its budget the same way a
//! run-to-completion does.
//!
//! One honest caveat about *intermediate* readings: with slippage 200 the
//! machine batches machine-step costs and charges them every 200 steps, so a
//! mid-run `budget` is exact for builtin costs but lags for machine-step
//! costs.  The totals agree; the intermediate curve is a staircase.  This is
//! the chain's own accounting, and smoothing it would be our invention rather
//! than an observation.

use eyre::{eyre, Result};
use pallas_primitives::conway::Language;
use uplc::ast::{NamedDeBruijn, Program, Term};
use uplc::machine::cost_model::{CostModel, ExBudget};
use uplc::machine::{Context, Machine, MachineState};

/// Machine construction mirrored from `Program::eval_version`, so that a
/// stepped run is budget-comparable with the chain's own evaluation.
const CHAIN_SLIPPAGE: u32 = 200;

/// Which CEK transition the machine was in when the step was observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateKind {
    /// Evaluating a term against an environment and a continuation.
    Compute,
    /// Returning a value into a continuation frame.
    Return,
    /// Terminal state; the result term is available.
    Done,
}

/// One observed CEK machine state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CekStep {
    /// Zero-based position of this state in the run.
    pub index: u64,
    /// The machine state's variant.
    pub kind: StateKind,
    /// Size of the environment carried by this state.  `Compute` carries an
    /// environment; `Return` and `Done` do not, and report `0`.
    pub env_len: usize,
    /// Depth of the continuation (number of frames above `NoFrame`).  `Done`
    /// carries no continuation and reports `0`.
    pub context_depth: usize,
    /// Budget *remaining* when this state was observed.
    pub budget: ExBudget,
}

/// The outcome of stepping a UPLC program to completion.
#[derive(Debug, Clone)]
pub struct CekRun {
    /// Every state the machine passed through, in order, ending with `Done`.
    pub steps: Vec<CekStep>,
    /// The term the program evaluated to.
    pub result: Term<NamedDeBruijn>,
    /// Budget the machine started with.
    pub initial_budget: ExBudget,
    /// Budget left when the machine reached `Done`.
    pub final_budget: ExBudget,
}

impl CekRun {
    /// Execution units consumed by the run: what the chain would have
    /// recorded for this script.
    pub fn consumed(&self) -> ExBudget {
        ExBudget {
            mem: self.initial_budget.mem - self.final_budget.mem,
            cpu: self.initial_budget.cpu - self.final_budget.cpu,
        }
    }

    /// Number of CEK transitions observed.
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }
}

/// Depth of a continuation: how many frames sit above `NoFrame`.
///
/// Walked iteratively rather than recursively because a deeply nested
/// validator would otherwise be able to overflow our stack while the machine
/// itself is perfectly happy — the recorder must not fail where the chain
/// succeeds.
fn context_depth(mut ctx: &Context) -> usize {
    let mut depth = 0;
    loop {
        ctx = match ctx {
            Context::NoFrame => return depth,
            Context::FrameAwaitArg(_, inner)
            | Context::FrameAwaitFunTerm(_, _, inner)
            | Context::FrameAwaitFunValue(_, inner)
            | Context::FrameForce(inner)
            | Context::FrameConstr(_, _, _, _, inner)
            | Context::FrameCases(_, _, inner) => inner,
        };
        depth += 1;
    }
}

/// Observe a machine state without consuming it.
///
/// `MachineState` is not `Clone` and `Machine::step` takes it by value, so the
/// facets are read through a borrow before the state is handed to `step`.
fn observe(index: u64, state: &MachineState, budget: ExBudget) -> CekStep {
    let (kind, env_len, context_depth_val) = match state {
        MachineState::Compute(ctx, env, _) => (StateKind::Compute, env.len(), context_depth(ctx)),
        MachineState::Return(ctx, _) => (StateKind::Return, 0, context_depth(ctx)),
        MachineState::Done(_) => (StateKind::Done, 0, 0),
    };

    CekStep {
        index,
        kind,
        env_len,
        context_depth: context_depth_val,
        budget,
    }
}

/// Step a UPLC program through the chain's CEK machine, recording every state.
///
/// This is the authoritative execution path: the machine is the `uplc` crate's
/// own, and every figure reported comes off it rather than from a
/// reinterpretation of the program.
pub fn step_program(
    program: Program<NamedDeBruijn>,
    version: &Language,
    initial_budget: ExBudget,
) -> Result<CekRun> {
    let mut machine = Machine::new(
        version.clone(),
        CostModel::default(),
        initial_budget,
        CHAIN_SLIPPAGE,
    );

    let mut state = machine
        .get_initial_machine_state(program.term)
        .map_err(|e| eyre!("CEK machine failed to start: {e:?}"))?;

    let mut steps = Vec::new();
    let mut index: u64 = 0;

    loop {
        let step = observe(index, &state, machine.ex_budget);
        steps.push(step);
        index += 1;

        match state {
            MachineState::Done(term) => {
                return Ok(CekRun {
                    steps,
                    result: term,
                    initial_budget,
                    final_budget: machine.ex_budget,
                });
            }
            live => {
                state = machine
                    .step(live)
                    .map_err(|e| eyre!("CEK machine failed at step {}: {e:?}", index - 1))?;
            }
        }
    }
}

/// How a replayed script's budget compares with the one consensus recorded.
///
/// This is the strongest control available to this recorder.  A Cardano
/// transaction carries its script, datum, redeemer and inputs, *and* the chain
/// records the execution units the script consumed.  Replaying the script and
/// comparing the two is a comparison no recorder bug can corrupt: the expected
/// value was produced by the network, not by us.  A test that compares the
/// recorder against itself — stepping against `run()`, say — proves only
/// internal consistency and is explicitly not this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetComparison {
    /// Units our stepped replay consumed.
    pub replayed: ExBudget,
    /// Units consensus recorded for the script.
    pub consensus: ExBudget,
}

impl BudgetComparison {
    pub fn new(replayed: ExBudget, consensus: ExBudget) -> Self {
        BudgetComparison {
            replayed,
            consensus,
        }
    }

    /// True when the replay consumed exactly what the chain recorded.
    pub fn agrees(&self) -> bool {
        self.replayed == self.consensus
    }

    /// Signed difference, replay minus consensus.  Zero on both axes iff
    /// `agrees()`.
    pub fn delta(&self) -> ExBudget {
        ExBudget {
            mem: self.replayed.mem - self.consensus.mem,
            cpu: self.replayed.cpu - self.consensus.cpu,
        }
    }
}

impl std::fmt::Display for BudgetComparison {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let delta = self.delta();
        write!(
            f,
            "replayed cpu={} mem={}; consensus cpu={} mem={}; delta cpu={:+} mem={:+} ({})",
            self.replayed.cpu,
            self.replayed.mem,
            self.consensus.cpu,
            self.consensus.mem,
            delta.cpu,
            delta.mem,
            if self.agrees() { "agree" } else { "DIVERGE" }
        )
    }
}
