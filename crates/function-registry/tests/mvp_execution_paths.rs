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
    FunctionExecutionPolicy, FunctionRecoverableError, FunctionRegistry, FunctionValue,
    ModelFunctionAdapter, RegistryError, register_mvp_namespace_contracts,
};

#[derive(Debug)]
struct StaticAdapter {
    key: String,
    response: Result<FunctionValue, FunctionRecoverableError>,
}

impl ModelFunctionAdapter for StaticAdapter {
    fn key(&self) -> &str {
        &self.key
    }

    fn invoke(
        &self,
        _function_name: &str,
        _args: &[FunctionValue],
        _policy: &FunctionExecutionPolicy,
    ) -> Result<FunctionValue, FunctionRecoverableError> {
        self.response.clone()
    }
}

#[test]
fn mvp_core_namespace_handlers_execute_expected_contract_behavior() {
    let mut registry = FunctionRegistry::new();
    register_mvp_namespace_contracts(&mut registry).expect("mvp contracts should register");

    let evidence = registry
        .invoke(
            "evidence.supporting_count",
            &[FunctionValue::String("abcdef".to_owned())],
            None,
        )
        .expect("evidence handler should execute");
    assert_eq!(evidence, FunctionValue::Integer(6));

    let confidence = registry
        .invoke("confidence.clamp", &[FunctionValue::Float(1.5)], None)
        .expect("confidence clamp should execute");
    assert_eq!(confidence, FunctionValue::Float(1.0));

    let temporal_equal = registry
        .invoke(
            "temporal.window_overlap_days",
            &[
                FunctionValue::String("2026-07-07".to_owned()),
                FunctionValue::String("2026-07-07".to_owned()),
            ],
            None,
        )
        .expect("temporal overlap should execute");
    assert_eq!(temporal_equal, FunctionValue::Integer(1));

    let temporal_distinct = registry
        .invoke(
            "temporal.window_overlap_days",
            &[
                FunctionValue::String("2026-07-07".to_owned()),
                FunctionValue::String("2026-07-08".to_owned()),
            ],
            None,
        )
        .expect("temporal overlap should execute");
    assert_eq!(temporal_distinct, FunctionValue::Integer(0));

    let coordination = registry
        .invoke(
            "coordination.window_score",
            &[FunctionValue::Integer(130)],
            None,
        )
        .expect("coordination clamp should execute");
    assert_eq!(coordination, FunctionValue::Integer(100));
}

#[test]
fn mvp_model_backed_handlers_map_recoverable_adapter_errors() {
    let mut registry = FunctionRegistry::new();
    register_mvp_namespace_contracts(&mut registry).expect("mvp contracts should register");

    let adapter = StaticAdapter {
        key: "crisis-adapter".to_owned(),
        response: Err(
            FunctionRecoverableError::new("MODEL_TIMEOUT", "temporary timeout", true)
                .expect("recoverable error should be valid"),
        ),
    };

    let error = registry
        .invoke(
            "crisis.classification_score",
            &[FunctionValue::String("flood response update".to_owned())],
            Some(&adapter),
        )
        .expect_err("recoverable adapter errors should map to registry error");

    assert!(matches!(
    error,
    RegistryError::FunctionExecutionRecoverable(recoverable)
    if recoverable.code == "MODEL_TIMEOUT"
    && recoverable.message == "temporary timeout"
    && recoverable.retryable
    ));
}
