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
use function_registry::{FunctionRegistry, register_mvp_namespace_contracts};

#[test]
fn mvp_namespace_contracts_register_expected_namespaces() {
    let mut registry = FunctionRegistry::new();

    register_mvp_namespace_contracts(&mut registry)
        .expect("mvp namespace contracts should register deterministically");

    let expected = [
        "evidence.supporting_count",
        "confidence.clamp",
        "temporal.window_overlap_days",
        "cti.observable_kind",
        "fimi.claim_similarity",
        "coordination.window_score",
        "crisis.classification_score",
    ];

    for function_name in expected {
        assert!(
            registry.get(function_name).is_some(),
            "function should be registered for namespace contract: {function_name}"
        );
    }
}

#[test]
fn mvp_model_backed_namespace_contracts_require_adapter_keys() {
    let mut registry = FunctionRegistry::new();
    register_mvp_namespace_contracts(&mut registry)
        .expect("mvp namespace contracts should register deterministically");

    let fimi = registry
        .get("fimi.claim_similarity")
        .expect("fimi namespace contract should be present");
    assert_eq!(fimi.model_adapter_key.as_deref(), Some("fimi-adapter"));

    let crisis = registry
        .get("crisis.classification_score")
        .expect("crisis namespace contract should be present");
    assert_eq!(crisis.model_adapter_key.as_deref(), Some("crisis-adapter"));
}

#[test]
fn mvp_namespace_contracts_reject_duplicate_registration() {
    let mut registry = FunctionRegistry::new();
    register_mvp_namespace_contracts(&mut registry).expect("first registration should succeed");

    let error = register_mvp_namespace_contracts(&mut registry)
        .expect_err("second registration should fail on duplicate names");

    assert!(matches!(
    error,
    function_registry::RegistryError::FunctionAlreadyRegistered(name)
    if name == "evidence.supporting_count"
    ));
}
