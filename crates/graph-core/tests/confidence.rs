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
use graph_core::*;

//
// Verify the public acceptance contract for the bounded confidence primitive.
// This test imports `Confidence` from the crate facade to ensure downstream
// callers can use `graph_core::*` without reaching into private modules.
//
// Given valid confidence values at both bounds and inside the range,
// when `Confidence::new` is called through the public API,
// then only values between `0.0` and `1.0` inclusive should be accepted.
#[test]
fn confidence_accepts_only_values_between_zero_and_one() {
    for value in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let confidence = Confidence::new(value).expect("bounded confidence should be accepted");

        assert_eq!(confidence.value(), value);
    }
}

//
// Verify the public rejection contract for invalid confidence values. This keeps
// the external API aligned with the domain invariant documented by issue 3.
//
// Given values outside the inclusive confidence range and `NaN`,
// when `Confidence::new` is called through the public API,
// then each invalid value should fail with `GraphError::InvalidConfidence`.
#[test]
fn confidence_rejects_invalid_values_from_public_api() {
    for value in [-0.01, 1.01] {
        let error = Confidence::new(value).expect_err("invalid confidence should be rejected");

        assert!(matches!(error, GraphError::InvalidConfidence(inner) if inner == value));
    }

    let error = Confidence::new(f64::NAN).expect_err("NaN confidence should be rejected");

    assert!(matches!(error, GraphError::InvalidConfidence(value) if value.is_nan()));
}
