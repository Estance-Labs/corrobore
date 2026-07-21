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
use function_registry::{FunctionSignature, FunctionValue, FunctionValueType, RegistryError};

#[test]
fn signature_validation_accepts_matching_argument_types() {
    let signature = FunctionSignature::new(
        vec![FunctionValueType::String, FunctionValueType::Integer],
        FunctionValueType::Boolean,
    )
    .expect("signature should be valid");

    let args = vec![
        FunctionValue::String("indicator--1".to_owned()),
        FunctionValue::Integer(7),
    ];

    signature
        .validate_arguments(&args)
        .expect("matching arguments should pass validation");
}

#[test]
fn signature_validation_rejects_arity_mismatch() {
    let signature = FunctionSignature::new(
        vec![FunctionValueType::String, FunctionValueType::Integer],
        FunctionValueType::Boolean,
    )
    .expect("signature should be valid");

    let args = vec![FunctionValue::String("indicator--1".to_owned())];

    let error = signature
        .validate_arguments(&args)
        .expect_err("arity mismatch should be rejected");

    assert!(matches!(
        error,
        RegistryError::ArgumentArityMismatch {
            expected: 2,
            actual: 1,
        }
    ));
}

#[test]
fn signature_validation_rejects_type_mismatch_with_index() {
    let signature = FunctionSignature::new(
        vec![FunctionValueType::String, FunctionValueType::Integer],
        FunctionValueType::Boolean,
    )
    .expect("signature should be valid");

    let args = vec![
        FunctionValue::String("indicator--1".to_owned()),
        FunctionValue::Boolean(true),
    ];

    let error = signature
        .validate_arguments(&args)
        .expect_err("type mismatch should be rejected");

    assert!(matches!(
        error,
        RegistryError::ArgumentTypeMismatch {
            index: 1,
            expected: FunctionValueType::Integer,
            actual: FunctionValueType::Boolean,
        }
    ));
}
