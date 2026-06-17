// Column-aware-steps fixture for FU-Column-Aware-Nav-Cardano.
//
// The Aiken hand-rolled parser used by the recorder is line-oriented
// (one statement per line), so this fixture exercises the column-bearing
// step emission by varying the indentation of statements: each `let`
// binding starts at a distinct 1-based byte column on its source line.
// The downstream test asserts that:
//
//   * `metadata.flags.has_column_aware_steps == true` (writer opted in)
//   * At least one step event surfaces a non-null `column` field
//     equal to the source line's first non-whitespace byte (>= 1)
fn compute() -> Int {
  let a = 10
    let b = 32
      let c = a + b
  c
}

test column_aware_test() {
  compute() == 42
}
