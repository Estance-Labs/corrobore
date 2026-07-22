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
use cypher_executor::{CypherPipelineExecutor, ExecutionError, ExecutionPolicy};
use function_registry::{
    FunctionCostClass, FunctionDeterminism, FunctionName, FunctionRegistry, FunctionSignature,
    FunctionSpec, FunctionValue, FunctionValueType, RegistryError,
};

fn count_handler(
    args: &[FunctionValue],
) -> Result<FunctionValue, function_registry::FunctionRecoverableError> {
    let first = args
        .first()
        .expect("signature validation should guarantee argument presence");
    match first {
        FunctionValue::String(input) => Ok(FunctionValue::Integer(input.len() as i64)),
        _ => unreachable!("signature validation should enforce string argument"),
    }
}

fn registry_with_count_function() -> FunctionRegistry {
    let mut registry = FunctionRegistry::new();
    let spec = FunctionSpec::new(
        FunctionName::new("evidence.supporting_count").expect("function name should be valid"),
        FunctionSignature::new(vec![FunctionValueType::String], FunctionValueType::Integer)
            .expect("signature should be valid"),
        FunctionDeterminism::Deterministic,
        FunctionCostClass::Low,
    )
    .expect("spec should be valid")
    .with_core_handler(count_handler);

    registry
        .register(spec)
        .expect("function should register deterministically");

    registry
}

#[test]
fn execute_registered_function_returns_typed_value() {
    let executor = CypherPipelineExecutor::new(ExecutionPolicy::strict_default());
    let registry = registry_with_count_function();

    let value = executor
        .execute_registered_function(
            &registry,
            "evidence.supporting_count",
            &[FunctionValue::String("abcd".to_owned())],
            None,
        )
        .expect("registered deterministic function should execute");

    assert_eq!(value, FunctionValue::Integer(4));
}

#[test]
fn execute_registered_function_maps_registry_errors() {
    let executor = CypherPipelineExecutor::new(ExecutionPolicy::strict_default());
    let registry = FunctionRegistry::new();

    let error = executor
        .execute_registered_function(
            &registry,
            "evidence.unknown",
            &[FunctionValue::String("abcd".to_owned())],
            None,
        )
        .expect_err("unknown function should map to execution error");

    assert!(matches!(
    error,
    ExecutionError::FunctionInvocation(RegistryError::FunctionNotFound(name))
    if name == "evidence.unknown"
    ));
}
