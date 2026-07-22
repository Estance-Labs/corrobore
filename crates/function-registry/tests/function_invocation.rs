// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
use function_registry::{
    FunctionCostClass, FunctionDeterminism, FunctionExecutionPolicy, FunctionName,
    FunctionRegistry, FunctionSignature, FunctionSpec, FunctionValue, FunctionValueType,
    ModelFunctionAdapter, RegistryError,
};

fn count_handler(
    args: &[FunctionValue],
) -> Result<FunctionValue, function_registry::FunctionRecoverableError> {
    let first = args.first().expect("validated by signature");
    match first {
        FunctionValue::String(input) => Ok(FunctionValue::Integer(input.len() as i64)),
        _ => unreachable!("signature validation should enforce string argument"),
    }
}

fn recoverable_handler(
    _: &[FunctionValue],
) -> Result<FunctionValue, function_registry::FunctionRecoverableError> {
    Err(function_registry::FunctionRecoverableError::new(
        "MODEL_TIMEOUT",
        "temporary timeout",
        true,
    )
    .expect("recoverable error should be valid"))
}

#[derive(Debug)]
struct MockModelAdapter {
    key: String,
}

impl ModelFunctionAdapter for MockModelAdapter {
    fn key(&self) -> &str {
        &self.key
    }

    fn invoke(
        &self,
        _function_name: &str,
        args: &[FunctionValue],
        _policy: &FunctionExecutionPolicy,
    ) -> Result<FunctionValue, function_registry::FunctionRecoverableError> {
        Ok(FunctionValue::Integer(args.len() as i64))
    }
}

fn deterministic_spec() -> FunctionSpec {
    FunctionSpec::new(
        FunctionName::new("evidence.supporting_count").expect("function name should be valid"),
        FunctionSignature::new(vec![FunctionValueType::String], FunctionValueType::Integer)
            .expect("signature should be valid"),
        FunctionDeterminism::Deterministic,
        FunctionCostClass::Low,
    )
    .expect("spec should be valid")
    .with_core_handler(count_handler)
}

#[test]
fn invoke_core_function_returns_typed_value() {
    let mut registry = FunctionRegistry::new();
    registry
        .register(deterministic_spec())
        .expect("function should register");

    let result = registry
        .invoke(
            "evidence.supporting_count",
            &[FunctionValue::String("abc".to_owned())],
            None,
        )
        .expect("invocation should succeed");

    assert_eq!(result, FunctionValue::Integer(3));
}

#[test]
fn invoke_returns_recoverable_error_from_handler() {
    let mut registry = FunctionRegistry::new();
    let spec = FunctionSpec::new(
        FunctionName::new("temporal.retryable_check").expect("function name should be valid"),
        FunctionSignature::new(vec![FunctionValueType::String], FunctionValueType::Integer)
            .expect("signature should be valid"),
        FunctionDeterminism::Deterministic,
        FunctionCostClass::Low,
    )
    .expect("spec should be valid")
    .with_core_handler(recoverable_handler);

    registry.register(spec).expect("function should register");

    let error = registry
        .invoke(
            "temporal.retryable_check",
            &[FunctionValue::String("abc".to_owned())],
            None,
        )
        .expect_err("recoverable handler error should bubble as typed error");

    assert!(matches!(
    error,
    RegistryError::FunctionExecutionRecoverable(recoverable)
    if recoverable.code == "MODEL_TIMEOUT" && recoverable.retryable
    ));
}

#[test]
fn invoke_rejects_unknown_function_name() {
    let registry = FunctionRegistry::new();

    let error = registry
        .invoke(
            "evidence.unknown",
            &[FunctionValue::String("abc".to_owned())],
            None,
        )
        .expect_err("unknown function should be rejected");

    assert!(matches!(
    error,
    RegistryError::FunctionNotFound(name) if name == "evidence.unknown"
    ));
}

#[test]
fn invoke_model_backed_function_requires_matching_adapter() {
    let mut registry = FunctionRegistry::new();
    let spec = FunctionSpec::new(
        FunctionName::new("fimi.claim_similarity").expect("function name should be valid"),
        FunctionSignature::new(
            vec![FunctionValueType::String, FunctionValueType::String],
            FunctionValueType::Integer,
        )
        .expect("signature should be valid"),
        FunctionDeterminism::NonDeterministic,
        FunctionCostClass::High,
    )
    .expect("spec should be valid")
    .with_model_adapter_key("fimi-adapter")
    .expect("adapter key should be valid");

    registry.register(spec).expect("function should register");

    let no_adapter_error = registry
        .invoke(
            "fimi.claim_similarity",
            &[
                FunctionValue::String("a".to_owned()),
                FunctionValue::String("b".to_owned()),
            ],
            None,
        )
        .expect_err("missing adapter should be rejected");
    assert!(matches!(
    no_adapter_error,
    RegistryError::MissingModelAdapter(name) if name == "fimi.claim_similarity"
    ));

    let wrong_adapter = MockModelAdapter {
        key: "cti-adapter".to_owned(),
    };
    let wrong_adapter_error = registry
        .invoke(
            "fimi.claim_similarity",
            &[
                FunctionValue::String("a".to_owned()),
                FunctionValue::String("b".to_owned()),
            ],
            Some(&wrong_adapter),
        )
        .expect_err("wrong adapter key should be rejected");

    assert!(matches!(
    wrong_adapter_error,
    RegistryError::ModelAdapterMismatch {
    function_name,
    expected_key,
    provided_key,
    } if function_name == "fimi.claim_similarity" && expected_key == "fimi-adapter" && provided_key == "cti-adapter"
    ));

    let correct_adapter = MockModelAdapter {
        key: "fimi-adapter".to_owned(),
    };
    let value = registry
        .invoke(
            "fimi.claim_similarity",
            &[
                FunctionValue::String("a".to_owned()),
                FunctionValue::String("b".to_owned()),
            ],
            Some(&correct_adapter),
        )
        .expect("matching adapter should execute function call");

    assert_eq!(value, FunctionValue::Integer(2));
}

#[test]
fn invoke_rejects_registered_core_function_without_handler() {
    let mut registry = FunctionRegistry::new();
    let spec = FunctionSpec::new(
        FunctionName::new("evidence.supporting_count").expect("function name should be valid"),
        FunctionSignature::new(vec![FunctionValueType::String], FunctionValueType::Integer)
            .expect("signature should be valid"),
        FunctionDeterminism::Deterministic,
        FunctionCostClass::Low,
    )
    .expect("spec should be valid");
    registry.register(spec).expect("function should register");

    let error = registry
        .invoke(
            "evidence.supporting_count",
            &[FunctionValue::String("abc".to_owned())],
            None,
        )
        .expect_err("missing deterministic core handler should be rejected");

    assert!(matches!(
    error,
    RegistryError::MissingCoreHandler(name) if name == "evidence.supporting_count"
    ));
}

#[test]
fn invoke_propagates_signature_arity_and_type_validation_errors() {
    let mut registry = FunctionRegistry::new();
    registry
        .register(deterministic_spec())
        .expect("function should register");

    let arity_error = registry
        .invoke("evidence.supporting_count", &[], None)
        .expect_err("arity mismatch should be propagated by invoke");
    assert!(matches!(
        arity_error,
        RegistryError::ArgumentArityMismatch {
            expected: 1,
            actual: 0,
        }
    ));

    let type_error = registry
        .invoke(
            "evidence.supporting_count",
            &[FunctionValue::Integer(42)],
            None,
        )
        .expect_err("type mismatch should be propagated by invoke");
    assert!(matches!(
        type_error,
        RegistryError::ArgumentTypeMismatch {
            index: 0,
            expected: FunctionValueType::String,
            actual: FunctionValueType::Integer,
        }
    ));
}
