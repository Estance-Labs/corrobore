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
use cypher_executor::{CypherPipelineExecutor, ExecutionPolicy};
use function_registry::{
    FunctionRegistry, FunctionValue, RegistryError, register_mvp_namespace_contracts,
};

#[test]
fn acceptance_deterministic_namespace_functions_return_stable_typed_outputs() {
    let executor = CypherPipelineExecutor::new(ExecutionPolicy::strict_default());
    let mut registry = FunctionRegistry::new();
    register_mvp_namespace_contracts(&mut registry)
        .expect("mvp namespace contracts should register deterministically");

    let evidence_value = executor
        .execute_registered_function(
            &registry,
            "evidence.supporting_count",
            &[FunctionValue::String("abcd".to_owned())],
            None,
        )
        .expect("evidence function should execute deterministically");
    assert_eq!(evidence_value, FunctionValue::Integer(4));

    let confidence_value = executor
        .execute_registered_function(
            &registry,
            "confidence.clamp",
            &[FunctionValue::Float(1.7)],
            None,
        )
        .expect("confidence function should execute deterministically");
    assert_eq!(confidence_value, FunctionValue::Float(1.0));

    let coordination_value = executor
        .execute_registered_function(
            &registry,
            "coordination.window_score",
            &[FunctionValue::Integer(123)],
            None,
        )
        .expect("coordination function should execute deterministically");
    assert_eq!(coordination_value, FunctionValue::Integer(100));
}

#[test]
fn acceptance_model_backed_contracts_return_typed_configuration_errors_without_adapter() {
    let executor = CypherPipelineExecutor::new(ExecutionPolicy::strict_default());
    let mut registry = FunctionRegistry::new();
    register_mvp_namespace_contracts(&mut registry)
        .expect("mvp namespace contracts should register deterministically");

    let error = executor
        .execute_registered_function(
            &registry,
            "fimi.claim_similarity",
            &[
                FunctionValue::String("claim-a".to_owned()),
                FunctionValue::String("claim-b".to_owned()),
            ],
            None,
        )
        .expect_err("missing model adapter should return typed error");

    assert!(matches!(
    error,
    cypher_executor::ExecutionError::FunctionInvocation(RegistryError::MissingModelAdapter(name))
    if name == "fimi.claim_similarity"
    ));
}
