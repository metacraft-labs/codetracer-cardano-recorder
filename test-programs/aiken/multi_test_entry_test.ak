// Exercises multiple `test` blocks per file.
//
// Pre-this-fixture the recorder only ran the FIRST `test` block found
// in the source, dropping every subsequent test on the floor.  A
// multi-test fixture is the simplest way to pin the new "execute every
// reachable test block" recorder behaviour: each `test` body must
// surface as its own Function entry with a balanced Call/Return event
// pair.
//
// The three tests exercise independent code paths inside the same
// file so a regression that drops one of them is immediately visible
// in the call-sequence assertion:
//   * `arithmetic_path()`     — pure UPLC-CEK addition (`add(a, b) == 7`).
//   * `pattern_match_path()`  — `when` arm dispatch over a Variant
//                               (`pick(Some(11)) == 11`).
//   * `pipeline_path()`       — pipe-operator desugaring through a
//                               two-stage chain (`5 |> double |> incr == 11`).
//
// Each test returns Bool (the comparison result); the test framework
// would normally check that result, but the recorder just executes
// the body and surfaces the chain of Call/Return events.

fn add(a: Int, b: Int) -> Int {
  let sum = a + b
  sum
}

fn pick(x: Option) -> Int {
  when x is {
    Some(n) -> n
    None -> 0
  }
}

fn double(x: Int) -> Int {
  let d = x * 2
  d
}

fn incr(x: Int) -> Int {
  let i = x + 1
  i
}

test arithmetic_path() {
  add(3, 4) == 7
}

test pattern_match_path() {
  pick(Some(11)) == 11
}

test pipeline_path() {
  let stage1 = 5 |> double()
  let stage2 = stage1 |> incr()
  stage2 == 11
}
