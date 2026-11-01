-- A hand-written fixture for the evaluator, spelling out by hand the arithmetic
-- of flow_test.ak.
-- Computes: a=10, b=32, sum_val=a+b=42, doubled=sum_val*2=84,
-- final_result=doubled+a=94, then checks final_result == 94
--
-- This is not what the compiler emits for flow_test.ak. The Aiken compiler
-- evaluates all of this at compile time and emits the constant True, a 6-byte
-- artifact with no computation in it, and its optimiser reduces this program
-- to the same constant. The arithmetic below exists only in this file.
--
-- The structure is:
--   equalsInteger(
--     addInteger(
--       multiplyInteger(addInteger(10, 32), 2),
--       10
--     ),
--     94
--   )
--
-- Which should evaluate to True (i.e., Constant::Bool(true))

(program 1.0.0
  [
    [
      (builtin equalsInteger)
      [
        [
          (builtin addInteger)
          [
            [
              (builtin multiplyInteger)
              [
                [
                  (builtin addInteger)
                  (con integer 10)
                ]
                (con integer 32)
              ]
            ]
            (con integer 2)
          ]
        ]
        (con integer 10)
      ]
    ]
    (con integer 94)
  ]
)
