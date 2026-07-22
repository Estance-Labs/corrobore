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
    FunctionCostClass, FunctionDeterminism, FunctionName, FunctionRegistry, FunctionSignature,
    FunctionSpec, FunctionValueType, RegistryError,
};

fn evidence_supporting_count_spec() -> FunctionSpec {
    FunctionSpec::new(
        FunctionName::new("evidence.supporting_count").expect("function name should be valid"),
        FunctionSignature::new(vec![FunctionValueType::String], FunctionValueType::Integer)
            .expect("signature should be valid"),
        FunctionDeterminism::Deterministic,
        FunctionCostClass::Low,
    )
    .expect("spec should be valid")
}

#[test]
fn register_and_lookup_namespaced_function() {
    let mut registry = FunctionRegistry::new();
    let spec = evidence_supporting_count_spec();

    registry
        .register(spec.clone())
        .expect("unique namespaced function should register");

    let registered = registry
        .get(spec.name.as_ref())
        .expect("registered function should be retrievable");

    assert_eq!(registered.name, spec.name);
    assert_eq!(registered.signature, spec.signature);
    assert_eq!(registered.determinism, FunctionDeterminism::Deterministic);
    assert_eq!(registered.cost_class, FunctionCostClass::Low);
}

#[test]
fn register_rejects_duplicate_function_name() {
    let mut registry = FunctionRegistry::new();
    let spec = evidence_supporting_count_spec();

    registry
        .register(spec.clone())
        .expect("first registration should succeed");

    let error = registry
        .register(spec)
        .expect_err("duplicate function name should be rejected");

    assert!(matches!(
    error,
    RegistryError::FunctionAlreadyRegistered(name) if name == "evidence.supporting_count"
    ));
}

#[test]
fn function_name_requires_namespace_separator() {
    let error =
        FunctionName::new("supporting_count").expect_err("function names must be namespaced");

    assert!(matches!(
    error,
    RegistryError::InvalidFunctionName(name) if name == "supporting_count"
    ));
}

#[test]
fn function_name_rejects_empty_segments_and_extra_separators() {
    let empty_namespace =
        FunctionName::new(".symbol").expect_err("function names should reject empty namespace");
    assert!(matches!(
    empty_namespace,
    RegistryError::InvalidFunctionName(name) if name == ".symbol"
    ));

    let empty_symbol =
        FunctionName::new("namespace.").expect_err("function names should reject empty symbol");
    assert!(matches!(
    empty_symbol,
    RegistryError::InvalidFunctionName(name) if name == "namespace."
    ));

    let extra_separator = FunctionName::new("namespace.symbol.extra")
        .expect_err("function names should reject more than one separator");
    assert!(matches!(
    extra_separator,
    RegistryError::InvalidFunctionName(name) if name == "namespace.symbol.extra"
    ));
}

#[test]
fn function_signature_requires_at_least_one_input() {
    let error = FunctionSignature::new(vec![], FunctionValueType::Integer)
        .expect_err("signature should require at least one input");

    assert!(matches!(error, RegistryError::MissingInputTypes));
}
