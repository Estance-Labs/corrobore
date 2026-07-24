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
//! Property and label storage primitives for graph records.
//!
//! Module boundary:
//! this module owns generic graph-core property value shapes and aliases used by
//! nodes and relationships. It must not own domain schemas, CTI object rules,
//! FIMI enrichment policy, or normalization beyond storage shape.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Typed property value that can be attached to graph records.
///
/// `PropertyValue` intentionally remains domain-neutral. It provides the common
/// scalar and ordered-list shapes needed by graph-core without embedding CTI,
/// FIMI, or crisis-specific property semantics.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PropertyValue {
    /// Explicitly present value with no payload.
    Null,
    /// Boolean scalar value.
    Bool(bool),
    /// Signed integer scalar value.
    Integer(i64),
    /// Floating point scalar value.
    Float(f64),
    /// Owned string scalar value.
    String(String),
    /// Ordered list of owned string values.
    StringList(Vec<String>),
    /// Ordered list of signed integer values.
    IntegerList(Vec<i64>),
    /// Ordered list of floating point values.
    FloatList(Vec<f64>),
    /// Ordered list of boolean values.
    BoolList(Vec<bool>),
    /// Arbitrarily nested, domain-neutral JSON value.
    ///
    /// Compatibility adapters use this escape hatch only when a field cannot be
    /// represented by the scalar and homogeneous list variants above. The graph
    /// core stores the value without assigning domain semantics to it.
    Json(serde_json::Value),
}

/// Map of graph record property names to typed property values.
pub type PropertyMap = HashMap<String, PropertyValue>;

/// Ordered graph record labels owned by node input and validation logic.
pub type LabelSet = Vec<String>;

#[cfg(test)]
mod tests {
    use super::*;

    //
    // Verify that the explicit null property variant is available for graph
    // records that need to represent an intentionally empty value.
    //
    // Given `PropertyValue::Null`,
    // when it is compared with the same variant,
    // then equality should hold.
    #[test]
    fn property_value_supports_null() {
        assert_eq!(PropertyValue::Null, PropertyValue::Null);
    }

    //
    // Verify that boolean property values preserve their boolean payload and keep
    // true and false distinct.
    //
    // Given boolean property values,
    // when they are compared,
    // then identical payloads should be equal and different payloads should differ.
    #[test]
    fn property_value_supports_bool() {
        assert_eq!(PropertyValue::Bool(true), PropertyValue::Bool(true));
        assert_ne!(PropertyValue::Bool(true), PropertyValue::Bool(false));
    }

    //
    // Verify that integer property values preserve signed integer payloads.
    //
    // Given an integer property value,
    // when it is compared with the same payload,
    // then equality should hold.
    #[test]
    fn property_value_supports_integer() {
        assert_eq!(PropertyValue::Integer(42), PropertyValue::Integer(42));
    }

    //
    // Verify that floating point property values preserve their numeric payload.
    //
    // Given a float property value,
    // when it is compared with the same payload,
    // then equality should hold.
    #[test]
    fn property_value_supports_float() {
        assert_eq!(PropertyValue::Float(4.2), PropertyValue::Float(4.2));
    }

    //
    // Verify that string property values preserve owned textual payloads.
    //
    // Given a string property value,
    // when it is compared with the same payload,
    // then equality should hold.
    #[test]
    fn property_value_supports_string() {
        assert_eq!(
            PropertyValue::String("APT28".to_owned()),
            PropertyValue::String("APT28".to_owned())
        );
    }

    //
    // Verify that string-list property values preserve ordered string collections.
    //
    // Given a string-list property value,
    // when it is compared with the same ordered payload,
    // then equality should hold.
    #[test]
    fn property_value_supports_string_list() {
        assert_eq!(
            PropertyValue::StringList(vec!["alpha".to_owned(), "beta".to_owned()]),
            PropertyValue::StringList(vec!["alpha".to_owned(), "beta".to_owned()])
        );
    }

    //
    // Verify that integer-list property values preserve ordered integer
    // collections.
    //
    // Given an integer-list property value,
    // when it is compared with the same ordered payload,
    // then equality should hold.
    #[test]
    fn property_value_supports_integer_list() {
        assert_eq!(
            PropertyValue::IntegerList(vec![1, 2, 3]),
            PropertyValue::IntegerList(vec![1, 2, 3])
        );
    }

    //
    // Verify that float-list property values preserve ordered float collections.
    //
    // Given a float-list property value,
    // when it is compared with the same ordered payload,
    // then equality should hold.
    #[test]
    fn property_value_supports_float_list() {
        assert_eq!(
            PropertyValue::FloatList(vec![1.0, 2.0, 3.5]),
            PropertyValue::FloatList(vec![1.0, 2.0, 3.5])
        );
    }

    //
    // Verify that boolean-list property values preserve ordered boolean
    // collections.
    //
    // Given a boolean-list property value,
    // when it is compared with the same ordered payload,
    // then equality should hold.
    #[test]
    fn property_value_supports_bool_list() {
        assert_eq!(
            PropertyValue::BoolList(vec![true, false, true]),
            PropertyValue::BoolList(vec![true, false, true])
        );
    }

    //
    // Verify that domain-neutral nested JSON survives as a typed property
    // without being flattened into strings or homogeneous lists.
    #[test]
    fn property_value_supports_nested_json() {
        let value = serde_json::json!({
            "extension": {
                "weights": [1, 2.5, 3],
                "enabled": true
            }
        });

        assert_eq!(
            PropertyValue::Json(value.clone()),
            PropertyValue::Json(value)
        );
    }

    //
    // Verify that the property map alias stores typed property values by string
    // key. Node and relationship inputs rely on this map for structured metadata.
    //
    // Given a `PropertyMap` with multiple typed values,
    // when values are retrieved by key,
    // then the original property values should be returned.
    #[test]
    fn property_map_stores_property_values_by_key() {
        let mut properties = PropertyMap::new();

        properties.insert("name".to_owned(), PropertyValue::String("APT28".to_owned()));
        properties.insert("score".to_owned(), PropertyValue::Integer(90));

        assert_eq!(
            properties.get("name"),
            Some(&PropertyValue::String("APT28".to_owned()))
        );
        assert_eq!(properties.get("score"), Some(&PropertyValue::Integer(90)));
    }

    //
    // Verify that the label set alias represents an ordered list of labels. Node
    // input validation owns the semantic validation, while this primitive alias
    // only defines storage shape.
    //
    // Given a `LabelSet` with two labels,
    // when it is compared with the same ordered labels,
    // then equality should hold.
    #[test]
    fn label_set_is_a_list_of_labels() {
        let labels: LabelSet = vec!["ThreatActor".to_owned(), "Campaign".to_owned()];

        assert_eq!(
            labels,
            vec!["ThreatActor".to_owned(), "Campaign".to_owned()]
        );
    }
}
