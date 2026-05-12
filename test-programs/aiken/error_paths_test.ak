// Exercises Aiken's error / fail / expect surfaces.
//
// `safe_compute()` runs a normal arithmetic chain so the recorder
// has a baseline of step events to emit before the failing path.
// `failing_compute()` triggers `fail` after computing a couple of
// let-bindings — the recorder must still surface the bindings that
// happened before the failure plus an error event for the fail.

fn safe_compute() -> Int {
  let a = 5
  let b = 7
  let c = a + b
  c
}

fn failing_compute() -> Int {
  let seen_a = 1
  let seen_b = 2
  fail @"intentional failure for trace coverage"
}

fn compute() -> Int {
  let safe_val = safe_compute()
  let bumped = safe_val + 100
  bumped
}

test safe_path() {
  compute() == 112
}

test failing_path() {
  failing_compute() == 0
}
