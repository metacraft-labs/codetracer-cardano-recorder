fn compute() -> Int {
  let a = 10
  let b = 32
  let sum_val = a + b
  let doubled = sum_val * 2
  let final_result = doubled + a
  final_result
}

test flow_test() {
  compute() == 94
}
