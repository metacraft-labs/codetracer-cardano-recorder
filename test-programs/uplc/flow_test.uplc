-- UPLC equivalent of flow_test.ak
-- Computes: a=10, b=32, sum_val=a+b=42, doubled=sum_val*2=84,
-- final_result=doubled+a=94, then checks final_result == 94
--
-- This is a manually constructed UPLC program that mirrors the
-- computation in the Aiken source flow_test.ak.
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
