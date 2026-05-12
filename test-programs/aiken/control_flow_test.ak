// Exercises if/else, when (pattern matching), and let-in chains.
//
// `compute()` selects a branch based on integer comparisons and
// pattern-matches on the resulting tag.  Each branch produces a
// distinct integer so the recorder must distinguish all three.
//
// The let chain is intentionally non-trivial: each binding depends
// on the previous one, which gives the recorder five separate step
// events to emit at five distinct source lines.

fn classify(n: Int) -> Int {
  if n < 0 {
    -1
  } else if n == 0 {
    0
  } else {
    1
  }
}

fn pick(tag: Int) -> Int {
  when tag is {
    -1 -> 100
    0 -> 200
    _ -> 300
  }
}

fn compute() -> Int {
  let raw = 7
  let sign = classify(raw)
  let bonus = pick(sign)
  let combined = sign + bonus
  let result = combined * 2
  result
}

test control_flow() {
  compute() == 602
}
