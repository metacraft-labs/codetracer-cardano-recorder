// Exercises recursion: tail-recursive accumulator + a second tail-
// recursive function in the same trace, both reached from the same
// `compute()` entry.  Aiken uses `when..is` for base-case pattern
// matching; this fixture exercises the recorder's when-arm matching
// (added alongside this fixture) which evaluates ONLY the arm whose
// pattern matches the scrutinee value, instead of the legacy
// "last arm wins" fall-through.
//
// `sum_acc(n, acc)` walks `n` down to zero, accumulating `acc + n`.
//   sum_acc(10, 0) = 1+2+...+10 = 55.
//
// `pow_acc(base, exp, acc)` walks `exp` down to zero, multiplying
// `acc` by `base` each step.  Tail-recursive variant so the recursive
// call is the whole arm RHS (the recorder's expression evaluator
// can't lift a function call out of `n * factorial(n - 1)`; the
// matching gap is captured by `recursion_test_factorial_form_ignored`).
//   pow_acc(2, 6, 1) = 2^6 = 64.
//
// `compute()` runs both and sums the results: 55 + 64 = 119.

fn sum_acc(n: Int, acc: Int) -> Int {
  when n is {
    0 -> acc
    _ -> sum_acc(n - 1, acc + n)
  }
}

fn pow_acc(base: Int, exp: Int, acc: Int) -> Int {
  when exp is {
    0 -> acc
    _ -> pow_acc(base, exp - 1, acc * base)
  }
}

fn compute() -> Int {
  let sum10 = sum_acc(10, 0)
  let pow64 = pow_acc(2, 6, 1)
  let combined = sum10 + pow64
  combined
}

test recursion() {
  compute() == 119
}
