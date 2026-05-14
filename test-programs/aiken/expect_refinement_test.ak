// Exercises Aiken's `expect <pattern> = <expr>` refinement-binding
// shape.  Semantically distinct from `let`: on a pattern-match
// failure the entire validator aborts (same shape as `fail`),
// making `expect` the canonical way to assert a refinement type
// on-chain.
//
// The recorder gained `Statement::Expect { pattern, expr }` and
// matching evaluator/static-sweep paths alongside this fixture:
//
//   * Runtime: `expect Some(x) = some_val` matches the `Some(x)`
//     pattern against the resolved RHS, binding `x` to the inner
//     payload on success.
//   * Static sweep (`emit_expect_events_for_program`): every
//     `expect` line in the program emits one
//     `EventLogKind::Error` io_event tagged `AikenExpectFailure`
//     so the failure surface is visible regardless of whether the
//     expect actually failed at recorder time.  Mirrors the
//     `Statement::Fail` static-sweep.
//
// Compute path (success only — the recorder doesn't reach the
// failure-case helper at runtime):
//
//   * `safe_extract()` runs `expect Some(x) = Some(13)` (success),
//     binds x=13, returns 13.
//   * `safe_field()` runs `expect Boxed { value } = Boxed { value: 29 }`
//     — but we only support single-payload constructor patterns, so
//     this fixture uses positional payload `Boxed(29)` instead.
//     Returns 29.
//   * `compute()` sums them: 13 + 29 = 42.
//
// `failing_extract()` is declared but never called: it carries an
// `expect Some(z) = None` line that contributes a static-sweep
// Error event but doesn't run.  This pins the "static-sweep covers
// unreached failure surfaces" guarantee.

type Boxed {
  Boxed(Int)
}

fn safe_extract() -> Int {
  let some_val = Some(13)
  expect Some(x) = some_val
  x
}

fn safe_field() -> Int {
  let boxed_val = Boxed(29)
  expect Boxed(value) = boxed_val
  value
}

fn failing_extract() -> Int {
  let none_val = None
  expect Some(z) = none_val
  z
}

fn compute() -> Int {
  let a = safe_extract()
  let b = safe_field()
  let total = a + b
  total
}

test expect_refinement() {
  compute() == 42
}
