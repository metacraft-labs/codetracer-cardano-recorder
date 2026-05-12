// Three-deep call chain.  `compute()` calls `outer()`, which calls
// `middle()`, which calls `inner()`.  The recorder must emit the
// calls in entry order (outer, middle, inner) and exit order
// (inner, middle, outer) — any reordering surfaces as a wrong
// `call_entry` sequence in the recorded trace.

fn inner() -> Int {
  let a = 1
  let b = 2
  let c = a + b
  c
}

fn middle() -> Int {
  let x = inner()
  let y = x + 10
  y
}

fn outer() -> Int {
  let p = middle()
  let q = p + 100
  q
}

fn compute() -> Int {
  let result = outer()
  result
}

test nested_calls() {
  compute() == 113
}
