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
    FunctionCachePolicy, FunctionExecutionPolicy, FunctionPermission, FunctionRecoverableError,
    FunctionValue, FunctionValueType, RegistryError,
};

#[test]
fn typed_value_reports_declared_runtime_type() {
    let value = FunctionValue::Integer(42);
    assert_eq!(value.value_type(), FunctionValueType::Integer);

    let list_value = FunctionValue::List(vec![FunctionValue::String("ioc".to_owned())]);
    assert_eq!(list_value.value_type(), FunctionValueType::List);
}

#[test]
fn typed_value_reports_all_declared_runtime_types() {
    assert_eq!(
        FunctionValue::String("indicator".to_owned()).value_type(),
        FunctionValueType::String
    );
    assert_eq!(
        FunctionValue::Float(0.42).value_type(),
        FunctionValueType::Float
    );
    assert_eq!(
        FunctionValue::Boolean(true).value_type(),
        FunctionValueType::Boolean
    );
    assert_eq!(
        FunctionValue::Timestamp("2026-07-07T00:00:00Z".to_owned()).value_type(),
        FunctionValueType::Timestamp
    );
    assert_eq!(
        FunctionValue::Object(std::collections::HashMap::new()).value_type(),
        FunctionValueType::Object
    );
    assert_eq!(FunctionValue::Null.value_type(), FunctionValueType::Null);
}

#[test]
fn recoverable_error_requires_non_empty_code() {
    let error = FunctionRecoverableError::new("", "temporary timeout", true)
        .expect_err("empty recoverable error code should be rejected");

    assert!(matches!(
    error,
    RegistryError::InvalidRecoverableErrorCode(code) if code.is_empty()
    ));
}

#[test]
fn recoverable_error_requires_non_empty_message_and_trims_inputs() {
    let error = FunctionRecoverableError::new("TIMEOUT", " ", true)
        .expect_err("empty recoverable error message should be rejected");

    assert!(matches!(
    error,
    RegistryError::InvalidRecoverableErrorMessage(message) if message.is_empty()
    ));

    let valid = FunctionRecoverableError::new(" MODEL_TIMEOUT ", " temporary timeout ", true)
        .expect("trimmed code/message should be accepted");
    assert_eq!(valid.code, "MODEL_TIMEOUT");
    assert_eq!(valid.message, "temporary timeout");
}

#[test]
fn execution_policy_represents_timeout_cache_and_permissions() {
    let policy = FunctionExecutionPolicy::new(
        2_500,
        FunctionCachePolicy::TtlSeconds(60),
        vec![
            FunctionPermission::ReadGraph,
            FunctionPermission::ReadEvidence,
        ],
    )
    .expect("execution policy should be valid");

    assert_eq!(policy.timeout_ms, 2_500);
    assert_eq!(policy.cache_policy, FunctionCachePolicy::TtlSeconds(60));
    assert_eq!(policy.permissions.len(), 2);
}

#[test]
fn execution_policy_rejects_zero_timeout() {
    let error = FunctionExecutionPolicy::new(
        0,
        FunctionCachePolicy::NoCache,
        vec![FunctionPermission::ReadGraph],
    )
    .expect_err("zero timeout should be rejected");

    assert!(matches!(error, RegistryError::InvalidTimeoutMs(0)));
}

#[test]
fn default_core_execution_policy_uses_expected_defaults() {
    let policy = FunctionExecutionPolicy::default_core();

    assert_eq!(policy.timeout_ms, 1_000);
    assert_eq!(policy.cache_policy, FunctionCachePolicy::NoCache);
    assert_eq!(policy.permissions, vec![FunctionPermission::ReadGraph]);
}
