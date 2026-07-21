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
//! Bounded confidence primitive for graph records.
//!
//! Module boundary:
//! this module owns validation for confidence values carried by nodes and
//! relationships. It does not own scoring policy, ranking, trust modeling, or
//! analyst workflow rules.

use serde::{Deserialize, Serialize};

use crate::GraphError;

/// Validated confidence score used by graph-core records.
///
/// `Confidence` represents a finite value in the inclusive `0.0..=1.0` range.
/// It keeps raw floating point validation at the primitive boundary so node,
/// relationship, and graph operations can depend on already validated values.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Confidence(f64);

impl Confidence {
    /// Build a validated confidence value.
    ///
    /// This constructor is the only public way to create `Confidence` from a raw
    /// floating point value. It keeps confidence validation close to the domain
    /// primitive so the rest of graph-core does not need to manipulate raw
    /// floats for confidence semantics.
    ///
    ///
    /// 1. Accept `0.0` as the lower inclusive bound.
    /// 2. Accept `1.0` as the upper inclusive bound.
    /// 3. Accept any finite value between `0.0` and `1.0`.
    /// 4. Return `Ok(Self(value))` when validation succeeds.
    ///
    /// # Errors
    ///
    ///
    /// 1. Reject values below `0.0`.
    /// 2. Reject values above `1.0`.
    /// 3. Reject `NaN` because it cannot be ordered or compared safely.
    /// 4. Return `GraphError::InvalidConfidence(value)` when validation fails.
    pub fn new(value: f64) -> Result<Self, GraphError> {
        if value.is_nan() || !(0.0..=1.0).contains(&value) {
            return Err(GraphError::InvalidConfidence(value));
        }

        Ok(Self(value))
    }

    /// Return the inner confidence value.
    ///
    /// This method exposes the validated floating point value without changing
    /// the confidence primitive. `Confidence` is `Copy`, so taking `self` keeps
    /// the API cheap and simple for callers.
    ///
    ///
    /// 1. Return the inner `f64` value.
    /// 2. Do not allocate.
    /// 3. Do not normalize or transform the value.
    ///
    /// # Errors
    ///
    ///
    /// This method has no error case because invalid confidence values cannot be
    /// constructed through the public constructor.
    pub fn value(self) -> f64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    //
    // Verify that the confidence lower bound is accepted. A confidence of zero
    // represents no confidence while still being a valid bounded value.
    //
    // Given the lower bound value `0.0`,
    // when `Confidence::new` is called,
    // then construction should succeed and the stored value should be `0.0`.
    #[test]
    fn confidence_accepts_zero() {
        let confidence = Confidence::new(0.0).expect("zero confidence should be accepted");

        assert_eq!(confidence.value(), 0.0);
    }

    //
    // Verify that the confidence upper bound is accepted. A confidence of one
    // represents full confidence while still being inside the bounded range.
    //
    // Given the upper bound value `1.0`,
    // when `Confidence::new` is called,
    // then construction should succeed and the stored value should be `1.0`.
    #[test]
    fn confidence_accepts_one() {
        let confidence = Confidence::new(1.0).expect("full confidence should be accepted");

        assert_eq!(confidence.value(), 1.0);
    }

    //
    // Verify that values strictly inside the confidence range are accepted. Most
    // confidence values will be fractional scores produced by extraction or
    // analyst workflows.
    //
    // Given a value between `0.0` and `1.0`,
    // when `Confidence::new` is called,
    // then construction should succeed and preserve the original value.
    #[test]
    fn confidence_accepts_value_between_zero_and_one() {
        let confidence = Confidence::new(0.42).expect("bounded confidence should be accepted");

        assert_eq!(confidence.value(), 0.42);
    }

    //
    // Verify that negative confidence scores are rejected. Confidence is modeled
    // as a bounded probability-like value, so it cannot go below zero.
    //
    // Given a value below `0.0`,
    // when `Confidence::new` is called,
    // then construction should fail with `GraphError::InvalidConfidence(value)`.
    #[test]
    fn confidence_rejects_value_below_zero() {
        let error = Confidence::new(-0.01).expect_err("negative confidence should be rejected");

        assert!(matches!(error, GraphError::InvalidConfidence(value) if value == -0.01));
    }

    //
    // Verify that confidence scores greater than one are rejected. The upper
    // bound prevents callers from storing unbounded raw scores as confidence.
    //
    // Given a value above `1.0`,
    // when `Confidence::new` is called,
    // then construction should fail with `GraphError::InvalidConfidence(value)`.
    #[test]
    fn confidence_rejects_value_above_one() {
        let error = Confidence::new(1.01).expect_err("confidence above one should be rejected");

        assert!(matches!(error, GraphError::InvalidConfidence(value) if value == 1.01));
    }

    //
    // Verify that `NaN` is rejected. `NaN` is not safely comparable and would
    // break confidence ordering, filtering, and validation semantics.
    //
    // Given `f64::NAN`,
    // when `Confidence::new` is called,
    // then construction should fail with `GraphError::InvalidConfidence(NaN)`.
    #[test]
    fn confidence_rejects_nan() {
        let error = Confidence::new(f64::NAN).expect_err("NaN confidence should be rejected");

        assert!(matches!(error, GraphError::InvalidConfidence(value) if value.is_nan()));
    }

    //
    // Verify that reading confidence returns the validated raw value. This gives
    // callers an explicit escape hatch for serialization, reporting, and scoring.
    //
    // Given a valid `Confidence`,
    // when `value` is called,
    // then it should return the inner `f64` without transformation.
    #[test]
    fn value_returns_inner_value() {
        let confidence = Confidence::new(0.75).expect("bounded confidence should be accepted");

        assert_eq!(confidence.value(), 0.75);
    }

    //
    // Verify that confidence remains cheap to copy. Confidence is a small value
    // object that will be carried by graph records and relationship metadata.
    //
    // Given a valid `Confidence`,
    // when it is assigned to another variable,
    // then both values should remain usable and equal.
    #[test]
    fn confidence_is_copyable() {
        let confidence = Confidence::new(0.5).expect("bounded confidence should be accepted");
        let copied = confidence;

        assert_eq!(confidence, copied);
    }

    //
    // Verify that confidence satisfies serde serialization contracts. The actual
    // wire format is left to the caller, but the primitive must expose serde
    // traits for graph persistence and API boundaries.
    //
    // Given the `Confidence` type,
    // when serde trait bounds are required,
    // then `Confidence` should satisfy both `Serialize` and `Deserialize`.
    #[test]
    fn confidence_is_serializable() {
        fn assert_serializable<T: Serialize + for<'de> Deserialize<'de>>() {}

        assert_serializable::<Confidence>();
    }
}
