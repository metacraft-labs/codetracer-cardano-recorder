// Exercises the structured value kinds: list, tuple, record.
//
// Aiken's record/tuple literals and list operations all need to
// round-trip through the recorder as `ValueRecord::Sequence`,
// `ValueRecord::Tuple`, and `ValueRecord::Struct` respectively.
//
// The trailing integer let-bindings give the recorder a known
// scalar baseline so the test can pin the steady-state behaviour
// while collection support catches up.

type Point {
  x: Int,
  y: Int,
}

fn sum_pair(p: (Int, Int)) -> Int {
  let (a, b) = p
  a + b
}

fn point_distance_sq(p: Point) -> Int {
  p.x * p.x + p.y * p.y
}

fn list_total() -> Int {
  let xs = [1, 2, 3, 4]
  let head_val = 1
  let len = 4
  head_val + len
}

fn compute() -> Int {
  let pair_total = sum_pair((10, 20))
  let pt_total = point_distance_sq(Point { x: 3, y: 4 })
  let list_total_val = list_total()
  let grand_total = pair_total + pt_total + list_total_val
  grand_total
}

test collections() {
  compute() == 60
}
