// Exercises Aiken's variant / sum-type constructors.
//
// `Option<a>` and `Result<a, b>` are the two built-in sum types used
// everywhere in Aiken validators (e.g. `expect Some(Datum { ... }) =
// datum`).  This fixture uses the user-defined `Status` analogue so
// the recorder doesn't have to special-case the standard-library
// constructor names — `Pending`, `Active(Int)`, `Failed` follow the
// same shape as `None`, `Some(x)`, `Error`.
//
// Each variant flows through a let-binding so the trace must
// surface it as `ValueRecord::Variant { discriminator, contents }`.
// The grand-total scalar drives the test's `compute() == 8`
// assertion through the existing UPLC-CEK arithmetic pipeline.

type Status {
  Pending
  Active(Int)
  Failed
}

fn classify(s: Status) -> Int {
  when s is {
    Pending -> 1
    Active(n) -> n
    Failed -> 0
  }
}

fn compute() -> Int {
  let pending_status = Pending
  let active_status = Active(5)
  let failed_status = Failed
  let pending_score = classify(pending_status)
  let active_score = classify(active_status)
  let failed_score = classify(failed_status)
  let total = pending_score + active_score + failed_score
  total
}

test variant_constructors() {
  compute() == 6
}
