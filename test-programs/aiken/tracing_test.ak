// Exercises Aiken's `trace` expression.  Each labelled trace call
// must surface as a `RecordEvent` (write kind) in the recorded
// trace — they're Aiken's only built-in I/O surface and therefore
// the closest analogue of `stdout` for this recorder.

fn compute() -> Int {
  let a = 4
  trace @"after-a": a
  let b = 5
  trace @"after-b": b
  let sum_val = a + b
  trace @"final-sum": sum_val
  sum_val
}

test tracing() {
  compute() == 9
}
