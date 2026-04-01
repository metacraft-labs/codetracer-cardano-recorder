//! On-chain transaction replay infrastructure.
//!
//! Provides types and functions for fetching Cardano transactions,
//! extracting their Plutus scripts, and reconstructing fully-applied
//! UPLC programs ready for CEK machine evaluation.
//!
//! Cardano validators are applied as successive UPLC `Apply` operations:
//!
//! ```text
//! ((script datum) redeemer) script_context   -- spending validators (V1/V2)
//! (script redeemer) script_context            -- minting policies, V3 validators
//! ```

use std::fmt;

use eyre::{eyre, Result};
use uplc::ast::{DeBruijn, NamedDeBruijn, Program};

use pallas_primitives::conway::Language;

// ---------------------------------------------------------------------------
// PlutusVersion
// ---------------------------------------------------------------------------

/// Plutus language version of an on-chain script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlutusVersion {
    V1,
    V2,
    V3,
}

impl fmt::Display for PlutusVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlutusVersion::V1 => write!(f, "PlutusV1"),
            PlutusVersion::V2 => write!(f, "PlutusV2"),
            PlutusVersion::V3 => write!(f, "PlutusV3"),
        }
    }
}

impl PlutusVersion {
    /// Convert to the `pallas_primitives` `Language` enum used by the UPLC
    /// evaluator for cost-model selection.
    pub fn to_language(&self) -> Language {
        match self {
            PlutusVersion::V1 => Language::PlutusV1,
            PlutusVersion::V2 => Language::PlutusV2,
            PlutusVersion::V3 => Language::PlutusV3,
        }
    }
}

// ---------------------------------------------------------------------------
// TransactionData
// ---------------------------------------------------------------------------

/// All the data needed to replay a Plutus validator execution from an
/// on-chain transaction.
#[derive(Debug, Clone)]
pub struct TransactionData {
    /// CBOR-encoded Plutus script bytes (the "compiled code" envelope).
    pub script_bytes: Vec<u8>,

    /// Optional datum (PlutusData encoded as hex string).
    /// Present for spending validators on V1/V2; absent for minting
    /// policies and V3 validators that receive datum inline.
    pub datum: Option<String>,

    /// Redeemer (PlutusData encoded as hex string).
    pub redeemer: String,

    /// ScriptContext (PlutusData encoded as hex string).
    pub script_context: String,

    /// Plutus language version of the script.
    pub script_version: PlutusVersion,
}

// ---------------------------------------------------------------------------
// Program reconstruction
// ---------------------------------------------------------------------------

/// Decode CBOR-encoded script bytes into a `Program<NamedDeBruijn>`.
///
/// The script bytes are expected to be a CBOR-wrapped flat-encoded UPLC
/// program, which is the standard on-chain format.
pub fn decode_script(script_bytes: &[u8]) -> Result<Program<NamedDeBruijn>> {
    let mut cbor_buffer = Vec::new();
    let db_program = Program::<DeBruijn>::from_cbor(script_bytes, &mut cbor_buffer)
        .map_err(|e| eyre!("failed to decode UPLC script from CBOR: {e}"))?;
    // Convert from DeBruijn to NamedDeBruijn (adds synthetic names).
    let program: Program<NamedDeBruijn> = db_program.into();
    Ok(program)
}

/// Decode a hex-encoded PlutusData string into `PlutusData`.
fn decode_plutus_data_hex(hex_str: &str) -> Result<pallas_primitives::conway::PlutusData> {
    use pallas_primitives::Fragment;
    let bytes = hex::decode(hex_str).map_err(|e| eyre!("invalid hex in PlutusData: {e}"))?;
    let data = pallas_primitives::conway::PlutusData::decode_fragment(&bytes)
        .map_err(|e| eyre!("failed to decode PlutusData from CBOR: {e}"))?;
    Ok(data)
}

/// Reconstruct a fully-applied UPLC program from transaction data.
///
/// This performs the successive `Apply` operations that the Cardano ledger
/// would perform before executing the validator:
///
/// - For spending validators (datum is `Some`):
///   `((script datum) redeemer) script_context`
/// - For minting policies / V3 validators (datum is `None`):
///   `(script redeemer) script_context`
///
/// Returns the program as a pretty-printed UPLC text string ready for
/// inspection or evaluation.
pub fn reconstruct_applied_program(tx_data: &TransactionData) -> Result<Program<NamedDeBruijn>> {
    // 1. Decode the script from CBOR bytes.
    let program = decode_script(&tx_data.script_bytes)?;

    // 2. Apply datum (if present -- spending validators).
    let program = if let Some(ref datum_hex) = tx_data.datum {
        let datum = decode_plutus_data_hex(datum_hex)?;
        program.apply_data(datum)
    } else {
        program
    };

    // 3. Apply redeemer.
    let redeemer = decode_plutus_data_hex(&tx_data.redeemer)?;
    let program = program.apply_data(redeemer);

    // 4. Apply script context.
    let script_context = decode_plutus_data_hex(&tx_data.script_context)?;
    let program = program.apply_data(script_context);

    Ok(program)
}

/// Convenience wrapper: reconstruct and return the pretty-printed UPLC text.
pub fn reconstruct_applied_program_text(tx_data: &TransactionData) -> Result<String> {
    let program = reconstruct_applied_program(tx_data)?;
    Ok(format!("{program}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use num_bigint::BigInt;
    use std::rc::Rc;
    use uplc::ast::{Constant, Data, DeBruijn, Program, Term};
    use uplc::machine::cost_model::ExBudget;

    /// Helper to construct a NamedDeBruijn value wrapped in Rc.
    fn named_db(name: &str, index: usize) -> Rc<NamedDeBruijn> {
        Rc::new(NamedDeBruijn {
            text: name.to_string(),
            index: DeBruijn::new(index),
        })
    }

    /// Build a trivial Plutus validator that ignores its arguments and
    /// returns `True` (a Constr 1 [] in Plutus).
    ///
    /// In UPLC this is: `\datum -> \redeemer -> \ctx -> (con bool True)`
    /// Encoded as a DeBruijn program: three nested lambdas wrapping a
    /// constant True.
    fn make_always_succeeds_script_cbor() -> Vec<u8> {
        let term: Term<NamedDeBruijn> = Term::Constant(Rc::new(Constant::Bool(true)));
        // Wrap in 3 lambdas: \d \r \ctx -> True
        let term = Term::Lambda {
            parameter_name: named_db("ctx", 0),
            body: Rc::new(term),
        };
        let term = Term::Lambda {
            parameter_name: named_db("redeemer", 0),
            body: Rc::new(term),
        };
        let term = Term::Lambda {
            parameter_name: named_db("datum", 0),
            body: Rc::new(term),
        };

        let program: Program<NamedDeBruijn> = Program {
            version: (1, 0, 0),
            term,
        };

        // Convert to DeBruijn for serialization, then to CBOR.
        let db_program: Program<DeBruijn> = program.into();
        db_program
            .to_cbor()
            .expect("failed to serialize test program to CBOR")
    }

    /// Build a Plutus validator that adds datum and redeemer integers:
    /// `\datum -> \redeemer -> \ctx -> addInteger(unIData(datum), unIData(redeemer))`
    fn make_add_datum_redeemer_script_cbor() -> Vec<u8> {
        use uplc::builtins::DefaultFunction;

        // Build: addInteger(unIData(var 3), unIData(var 2))
        // In DeBruijn, var 3 = datum (outermost lambda), var 2 = redeemer, var 1 = ctx
        let un_i_data_datum = Term::Apply {
            function: Rc::new(Term::Builtin(DefaultFunction::UnIData)),
            argument: Rc::new(Term::Var(named_db("datum", 3))),
        };
        let un_i_data_redeemer = Term::Apply {
            function: Rc::new(Term::Builtin(DefaultFunction::UnIData)),
            argument: Rc::new(Term::Var(named_db("redeemer", 2))),
        };
        let add_term = Term::Apply {
            function: Rc::new(Term::Apply {
                function: Rc::new(Term::Builtin(DefaultFunction::AddInteger)),
                argument: Rc::new(un_i_data_datum),
            }),
            argument: Rc::new(un_i_data_redeemer),
        };

        // Wrap in 3 lambdas
        let term = Term::Lambda {
            parameter_name: named_db("ctx", 0),
            body: Rc::new(add_term),
        };
        let term = Term::Lambda {
            parameter_name: named_db("redeemer", 0),
            body: Rc::new(term),
        };
        let term = Term::Lambda {
            parameter_name: named_db("datum", 0),
            body: Rc::new(term),
        };

        let program: Program<NamedDeBruijn> = Program {
            version: (1, 0, 0),
            term,
        };

        let db_program: Program<DeBruijn> = program.into();
        db_program
            .to_cbor()
            .expect("failed to serialize test program to CBOR")
    }

    /// Encode a PlutusData integer as hex.
    fn plutus_data_int_hex(n: i64) -> String {
        let data = Data::integer(BigInt::from(n));
        Data::to_hex(data)
    }

    /// Encode an empty Constr 0 as hex (used as a dummy script context).
    fn plutus_data_unit_hex() -> String {
        let data = Data::constr(0, vec![]);
        Data::to_hex(data)
    }

    #[test]
    fn test_transaction_data_construction() {
        let tx = TransactionData {
            script_bytes: vec![1, 2, 3],
            datum: Some("00".to_string()),
            redeemer: "01".to_string(),
            script_context: "d87980".to_string(),
            script_version: PlutusVersion::V2,
        };
        assert_eq!(tx.script_version, PlutusVersion::V2);
        assert!(tx.datum.is_some());
    }

    #[test]
    fn test_plutus_version_display() {
        assert_eq!(PlutusVersion::V1.to_string(), "PlutusV1");
        assert_eq!(PlutusVersion::V2.to_string(), "PlutusV2");
        assert_eq!(PlutusVersion::V3.to_string(), "PlutusV3");
    }

    #[test]
    fn test_plutus_version_to_language() {
        assert_eq!(PlutusVersion::V1.to_language(), Language::PlutusV1);
        assert_eq!(PlutusVersion::V2.to_language(), Language::PlutusV2);
        assert_eq!(PlutusVersion::V3.to_language(), Language::PlutusV3);
    }

    #[test]
    fn test_decode_script_roundtrip() {
        let cbor = make_always_succeeds_script_cbor();
        let program = decode_script(&cbor).expect("decode failed");
        let pretty = format!("{program}");
        assert!(
            pretty.contains("lam"),
            "expected lambda in pretty-printed output, got: {pretty}"
        );
    }

    #[test]
    fn test_reconstruct_always_succeeds() {
        let cbor = make_always_succeeds_script_cbor();
        let datum_hex = plutus_data_unit_hex();
        let redeemer_hex = plutus_data_unit_hex();
        let ctx_hex = plutus_data_unit_hex();

        let tx_data = TransactionData {
            script_bytes: cbor,
            datum: Some(datum_hex),
            redeemer: redeemer_hex,
            script_context: ctx_hex,
            script_version: PlutusVersion::V2,
        };

        let program = reconstruct_applied_program(&tx_data).expect("reconstruction failed");

        // Evaluate through the CEK machine -- should return True.
        let result = program.eval(ExBudget::default());
        let term = result.result().expect("evaluation failed");

        assert_eq!(
            term,
            Term::Constant(Rc::new(Constant::Bool(true))),
            "always-succeeds validator should return True"
        );
    }

    #[test]
    fn test_reconstruct_without_datum() {
        // Minting-policy style: only redeemer + context (2 lambdas).
        let term: Term<NamedDeBruijn> = Term::Constant(Rc::new(Constant::Bool(true)));
        let term = Term::Lambda {
            parameter_name: named_db("ctx", 0),
            body: Rc::new(term),
        };
        let term = Term::Lambda {
            parameter_name: named_db("redeemer", 0),
            body: Rc::new(term),
        };

        let program: Program<NamedDeBruijn> = Program {
            version: (1, 0, 0),
            term,
        };
        let db_program: Program<DeBruijn> = program.into();
        let cbor = db_program.to_cbor().unwrap();

        let redeemer_hex = plutus_data_unit_hex();
        let ctx_hex = plutus_data_unit_hex();

        let tx_data = TransactionData {
            script_bytes: cbor,
            datum: None,
            redeemer: redeemer_hex,
            script_context: ctx_hex,
            script_version: PlutusVersion::V2,
        };

        let program = reconstruct_applied_program(&tx_data).expect("reconstruction failed");
        let result = program.eval(ExBudget::default());
        let term = result.result().expect("evaluation failed");
        assert_eq!(term, Term::Constant(Rc::new(Constant::Bool(true))));
    }

    #[test]
    fn test_reconstruct_add_datum_redeemer() {
        let cbor = make_add_datum_redeemer_script_cbor();

        let datum_hex = plutus_data_int_hex(10);
        let redeemer_hex = plutus_data_int_hex(32);
        let ctx_hex = plutus_data_unit_hex();

        let tx_data = TransactionData {
            script_bytes: cbor,
            datum: Some(datum_hex),
            redeemer: redeemer_hex,
            script_context: ctx_hex,
            script_version: PlutusVersion::V2,
        };

        let program = reconstruct_applied_program(&tx_data).expect("reconstruction failed");

        let result = program.eval(ExBudget::default());
        let term = result.result().expect("evaluation failed");

        // 10 + 32 = 42
        assert_eq!(
            term,
            Term::Constant(Rc::new(Constant::Integer(BigInt::from(42)))),
            "add-datum-redeemer validator should return 42"
        );
    }

    #[test]
    fn test_reconstruct_applied_program_text() {
        let cbor = make_always_succeeds_script_cbor();
        let datum_hex = plutus_data_unit_hex();
        let redeemer_hex = plutus_data_unit_hex();
        let ctx_hex = plutus_data_unit_hex();

        let tx_data = TransactionData {
            script_bytes: cbor,
            datum: Some(datum_hex),
            redeemer: redeemer_hex,
            script_context: ctx_hex,
            script_version: PlutusVersion::V2,
        };

        let text = reconstruct_applied_program_text(&tx_data).expect("reconstruction failed");

        // The text should contain the applied program.
        assert!(!text.is_empty(), "program text should not be empty");
    }

    #[test]
    fn test_decode_plutus_data_hex_integer() {
        let hex = plutus_data_int_hex(42);
        let data = decode_plutus_data_hex(&hex).expect("decode failed");
        // Should be a BigInt variant.
        match data {
            pallas_primitives::conway::PlutusData::BigInt(_) => {}
            other => panic!("expected BigInt, got: {other:?}"),
        }
    }

    #[test]
    fn test_invalid_script_bytes() {
        let result = decode_script(&[0xFF, 0xFF]);
        assert!(result.is_err(), "should fail on invalid CBOR");
    }

    #[test]
    fn test_invalid_plutus_data_hex() {
        let result = decode_plutus_data_hex("not_valid_hex");
        assert!(result.is_err(), "should fail on invalid hex");
    }
}
