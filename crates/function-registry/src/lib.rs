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
#![warn(missing_docs)]

//! Typed function registry contracts for Cypher-facing built-ins.

use std::collections::HashMap;

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
/// Registry error.
pub enum RegistryError {
    #[error("invalid function name: {0}")]
    /// Invalid function name.
    InvalidFunctionName(String),
    #[error("function already registered: {0}")]
    /// Function already registered.
    FunctionAlreadyRegistered(String),
    #[error("function signature must declare at least one input type")]
    /// Missing input types.
    MissingInputTypes,
    #[error("invalid recoverable error code: {0}")]
    /// Invalid recoverable error code.
    InvalidRecoverableErrorCode(String),
    #[error("invalid recoverable error message: {0}")]
    /// Invalid recoverable error message.
    InvalidRecoverableErrorMessage(String),
    #[error("invalid function timeout in milliseconds: {0}")]
    /// Invalid timeout ms.
    InvalidTimeoutMs(u64),
    #[error("function argument arity mismatch: expected {expected}, got {actual}")]
    /// Argument arity mismatch.
    ArgumentArityMismatch {
        /// Expected.
        expected: usize,
        /// Actual.
        actual: usize,
    },
    #[error(
        "function argument type mismatch at index {index}: expected {expected:?}, got {actual:?}"
    )]
    /// Argument type mismatch.
    ArgumentTypeMismatch {
        /// Index.
        index: usize,
        /// Expected.
        expected: FunctionValueType,
        /// Actual.
        actual: FunctionValueType,
    },
    #[error("function not found: {0}")]
    /// Function not found.
    FunctionNotFound(String),
    #[error("missing model adapter for function: {0}")]
    /// Missing model adapter.
    MissingModelAdapter(String),
    #[error(
        "model adapter mismatch for function {function_name}: expected {expected_key}, got {provided_key}"
    )]
    /// Model adapter mismatch.
    ModelAdapterMismatch {
        /// Function name.
        function_name: String,
        /// Expected key.
        expected_key: String,
        /// Provided key.
        provided_key: String,
    },
    #[error("missing deterministic core handler for function: {0}")]
    /// Missing core handler.
    MissingCoreHandler(String),
    #[error("invalid model adapter key: {0}")]
    /// Invalid model adapter key.
    InvalidModelAdapterKey(String),
    #[error("function execution returned recoverable error: {0}")]
    /// Function execution recoverable.
    FunctionExecutionRecoverable(FunctionRecoverableError),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
/// Function name.
pub struct FunctionName(String);

impl FunctionName {
    /// Creates a new instance.
    pub fn new(value: &str) -> Result<Self, RegistryError> {
        let trimmed = value.trim();
        if trimmed.is_empty() || !trimmed.contains('.') {
            return Err(RegistryError::InvalidFunctionName(trimmed.to_owned()));
        }

        let mut segments = trimmed.split('.');
        let namespace = segments.next().unwrap_or_default();
        let symbol = segments.next().unwrap_or_default();
        if namespace.is_empty() || symbol.is_empty() || segments.next().is_some() {
            return Err(RegistryError::InvalidFunctionName(trimmed.to_owned()));
        }

        Ok(Self(trimmed.to_owned()))
    }

    /// Returns the value as str.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for FunctionName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Function value type.
pub enum FunctionValueType {
    /// String.
    String,
    /// Integer.
    Integer,
    /// Float.
    Float,
    /// Boolean.
    Boolean,
    /// Timestamp.
    Timestamp,
    /// List.
    List,
    /// Object.
    Object,
    /// Null.
    Null,
}

#[derive(Clone, Debug, PartialEq)]
/// Function value.
pub enum FunctionValue {
    /// String.
    String(String),
    /// Integer.
    Integer(i64),
    /// Float.
    Float(f64),
    /// Boolean.
    Boolean(bool),
    /// Timestamp.
    Timestamp(String),
    /// List.
    List(Vec<FunctionValue>),
    /// Object.
    Object(HashMap<String, FunctionValue>),
    /// Null.
    Null,
}

impl FunctionValue {
    //
    // Runtime values report a coarse stable type so planner/executor boundaries
    // can validate contracts without inspecting variant payload internals.
    /// Value type.
    pub fn value_type(&self) -> FunctionValueType {
        match self {
            Self::String(_) => FunctionValueType::String,
            Self::Integer(_) => FunctionValueType::Integer,
            Self::Float(_) => FunctionValueType::Float,
            Self::Boolean(_) => FunctionValueType::Boolean,
            Self::Timestamp(_) => FunctionValueType::Timestamp,
            Self::List(_) => FunctionValueType::List,
            Self::Object(_) => FunctionValueType::Object,
            Self::Null => FunctionValueType::Null,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Function recoverable error.
pub struct FunctionRecoverableError {
    /// Code.
    pub code: String,
    /// Message.
    pub message: String,
    /// Retryable.
    pub retryable: bool,
}

impl std::fmt::Display for FunctionRecoverableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl FunctionRecoverableError {
    //
    // Recoverable function failures are explicit typed values so callers can
    // branch on stable error codes instead of string-matching opaque failures.
    /// Creates a new instance.
    pub fn new(code: &str, message: &str, retryable: bool) -> Result<Self, RegistryError> {
        let normalized_code = code.trim();
        if normalized_code.is_empty() {
            return Err(RegistryError::InvalidRecoverableErrorCode(
                normalized_code.to_owned(),
            ));
        }

        let normalized_message = message.trim();
        if normalized_message.is_empty() {
            return Err(RegistryError::InvalidRecoverableErrorMessage(
                normalized_message.to_owned(),
            ));
        }

        Ok(Self {
            code: normalized_code.to_owned(),
            message: normalized_message.to_owned(),
            retryable,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Function permission.
pub enum FunctionPermission {
    /// Read graph.
    ReadGraph,
    /// Read evidence.
    ReadEvidence,
    /// Model adapter.
    ModelAdapter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Function cache policy.
pub enum FunctionCachePolicy {
    /// No cache.
    NoCache,
    /// Ttl seconds.
    TtlSeconds(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Function execution policy.
pub struct FunctionExecutionPolicy {
    /// Timeout ms.
    pub timeout_ms: u64,
    /// Cache policy.
    pub cache_policy: FunctionCachePolicy,
    /// Permissions.
    pub permissions: Vec<FunctionPermission>,
}

impl FunctionExecutionPolicy {
    /// Default core.
    pub fn default_core() -> Self {
        Self {
            // Timeout ms.
            timeout_ms: 1_000,
            // Cache policy.
            cache_policy: FunctionCachePolicy::NoCache,
            // Permissions.
            permissions: vec![FunctionPermission::ReadGraph],
        }
    }

    //
    // Execution policy keeps deterministic metadata (timeout/cache/permissions)
    // attached to each registered function contract.
    /// Creates a new instance.
    pub fn new(
        timeout_ms: u64,
        cache_policy: FunctionCachePolicy,
        permissions: Vec<FunctionPermission>,
    ) -> Result<Self, RegistryError> {
        if timeout_ms == 0 {
            return Err(RegistryError::InvalidTimeoutMs(timeout_ms));
        }

        Ok(Self {
            timeout_ms,
            cache_policy,
            permissions,
        })
    }
}

/// Model function adapter.
pub trait ModelFunctionAdapter {
    /// Returns the adapter key.
    fn key(&self) -> &str;

    /// Invokes the model function with the given arguments and policy.
    fn invoke(
        &self,
        function_name: &str,
        args: &[FunctionValue],
        policy: &FunctionExecutionPolicy,
    ) -> Result<FunctionValue, FunctionRecoverableError>;
}

/// Type alias for [`CoreFunctionHandler`].
pub type CoreFunctionHandler =
    fn(&[FunctionValue]) -> Result<FunctionValue, FunctionRecoverableError>;

fn evidence_supporting_count(
    args: &[FunctionValue],
) -> Result<FunctionValue, FunctionRecoverableError> {
    let value = args
        .first()
        .expect("signature validation must ensure first argument");
    match value {
        FunctionValue::String(input) => Ok(FunctionValue::Integer(input.len() as i64)),
        _ => Err(FunctionRecoverableError::new(
            "TYPE_MISMATCH",
            "expected string argument for evidence.supporting_count",
            false,
        )
        .expect("static recoverable error should be valid")),
    }
}

fn confidence_clamp(args: &[FunctionValue]) -> Result<FunctionValue, FunctionRecoverableError> {
    let value = args
        .first()
        .expect("signature validation must ensure first argument");
    match value {
        FunctionValue::Float(input) => Ok(FunctionValue::Float(input.clamp(0.0, 1.0))),
        _ => Err(FunctionRecoverableError::new(
            "TYPE_MISMATCH",
            "expected float argument for confidence.clamp",
            false,
        )
        .expect("static recoverable error should be valid")),
    }
}

fn temporal_window_overlap_days(
    args: &[FunctionValue],
) -> Result<FunctionValue, FunctionRecoverableError> {
    let first = args
        .first()
        .expect("signature validation must ensure first argument");
    let second = args
        .get(1)
        .expect("signature validation must ensure second argument");
    match (first, second) {
        (FunctionValue::String(start), FunctionValue::String(end)) => {
            let overlap_days = if start == end { 1 } else { 0 };
            Ok(FunctionValue::Integer(overlap_days))
        }
        _ => Err(FunctionRecoverableError::new(
            "TYPE_MISMATCH",
            "expected string arguments for temporal.window_overlap_days",
            false,
        )
        .expect("static recoverable error should be valid")),
    }
}

fn coordination_window_score(
    args: &[FunctionValue],
) -> Result<FunctionValue, FunctionRecoverableError> {
    let value = args
        .first()
        .expect("signature validation must ensure first argument");
    match value {
        FunctionValue::Integer(input) => Ok(FunctionValue::Integer((*input).clamp(0, 100))),
        _ => Err(FunctionRecoverableError::new(
            "TYPE_MISMATCH",
            "expected integer argument for coordination.window_score",
            false,
        )
        .expect("static recoverable error should be valid")),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Function signature.
pub struct FunctionSignature {
    /// Input types.
    pub input_types: Vec<FunctionValueType>,
    /// Output type.
    pub output_type: FunctionValueType,
}

impl FunctionSignature {
    /// Creates a new instance.
    pub fn new(
        input_types: Vec<FunctionValueType>,
        output_type: FunctionValueType,
    ) -> Result<Self, RegistryError> {
        if input_types.is_empty() {
            return Err(RegistryError::MissingInputTypes);
        }

        Ok(Self {
            input_types,
            output_type,
        })
    }

    //
    // Signature validation enforces deterministic argument shape checks before a
    // function implementation executes.
    /// Validates the arguments.
    pub fn validate_arguments(&self, args: &[FunctionValue]) -> Result<(), RegistryError> {
        if args.len() != self.input_types.len() {
            return Err(RegistryError::ArgumentArityMismatch {
                expected: self.input_types.len(),
                actual: args.len(),
            });
        }

        for (index, (expected, actual_value)) in
            self.input_types.iter().zip(args.iter()).enumerate()
        {
            let actual = actual_value.value_type();
            if &actual != expected {
                return Err(RegistryError::ArgumentTypeMismatch {
                    index,
                    expected: expected.clone(),
                    actual,
                });
            }
        }

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Function determinism.
pub enum FunctionDeterminism {
    /// Deterministic.
    Deterministic,
    /// Non deterministic.
    NonDeterministic,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Function cost class.
pub enum FunctionCostClass {
    /// Low.
    Low,
    /// Medium.
    Medium,
    /// High.
    High,
}

#[derive(Clone, Debug)]
/// Function spec.
pub struct FunctionSpec {
    /// Name.
    pub name: FunctionName,
    /// Signature.
    pub signature: FunctionSignature,
    /// Determinism.
    pub determinism: FunctionDeterminism,
    /// Cost class.
    pub cost_class: FunctionCostClass,
    /// Execution policy.
    pub execution_policy: FunctionExecutionPolicy,
    /// Model adapter key.
    pub model_adapter_key: Option<String>,
    /// Core handler.
    pub core_handler: Option<CoreFunctionHandler>,
}

impl FunctionSpec {
    /// Creates a new instance.
    pub fn new(
        name: FunctionName,
        signature: FunctionSignature,
        determinism: FunctionDeterminism,
        cost_class: FunctionCostClass,
    ) -> Result<Self, RegistryError> {
        Ok(Self {
            name,
            signature,
            determinism,
            cost_class,
            execution_policy: FunctionExecutionPolicy::default_core(),
            model_adapter_key: None,
            core_handler: None,
        })
    }

    /// Sets the execution policy.
    pub fn with_execution_policy(mut self, policy: FunctionExecutionPolicy) -> Self {
        self.execution_policy = policy;
        self
    }

    /// Sets the core handler.
    pub fn with_core_handler(mut self, handler: CoreFunctionHandler) -> Self {
        self.core_handler = Some(handler);
        self
    }

    /// Sets the model adapter key.
    pub fn with_model_adapter_key(mut self, key: &str) -> Result<Self, RegistryError> {
        let normalized = key.trim();
        if normalized.is_empty() {
            return Err(RegistryError::InvalidModelAdapterKey(normalized.to_owned()));
        }

        self.model_adapter_key = Some(normalized.to_owned());
        Ok(self)
    }
}

#[derive(Clone, Debug, Default)]
/// Function registry.
pub struct FunctionRegistry {
    by_name: HashMap<String, FunctionSpec>,
}

impl FunctionRegistry {
    /// Creates a new instance.
    pub fn new() -> Self {
        Self {
            // By name.
            by_name: HashMap::new(),
        }
    }

    //
    // Registration enforces a unique namespace-qualified function identity and
    // will become the single admission point for built-in metadata validation.
    /// Register.
    pub fn register(&mut self, spec: FunctionSpec) -> Result<(), RegistryError> {
        let key = spec.name.as_str().to_owned();
        if self.by_name.contains_key(&key) {
            return Err(RegistryError::FunctionAlreadyRegistered(key));
        }

        self.by_name.insert(key, spec);
        Ok(())
    }

    //
    // Lookups return immutable function contracts so planner/executor boundaries
    // can validate calls without mutating registry state.
    /// Get.
    pub fn get(&self, name: &str) -> Option<&FunctionSpec> {
        self.by_name.get(name)
    }

    /// Invoke.
    pub fn invoke(
        &self,
        name: &str,
        args: &[FunctionValue],
        model_adapter: Option<&dyn ModelFunctionAdapter>,
    ) -> Result<FunctionValue, RegistryError> {
        let spec = self
            .by_name
            .get(name)
            .ok_or_else(|| RegistryError::FunctionNotFound(name.to_owned()))?;

        spec.signature.validate_arguments(args)?;

        if let Some(required_adapter_key) = &spec.model_adapter_key {
            let adapter =
                model_adapter.ok_or_else(|| RegistryError::MissingModelAdapter(name.to_owned()))?;

            if adapter.key() != required_adapter_key {
                return Err(RegistryError::ModelAdapterMismatch {
                    function_name: name.to_owned(),
                    expected_key: required_adapter_key.clone(),
                    provided_key: adapter.key().to_owned(),
                });
            }

            return adapter
                .invoke(name, args, &spec.execution_policy)
                .map_err(RegistryError::FunctionExecutionRecoverable);
        }

        let handler = spec
            .core_handler
            .ok_or_else(|| RegistryError::MissingCoreHandler(name.to_owned()))?;

        handler(args).map_err(RegistryError::FunctionExecutionRecoverable)
    }
}

/// Register mvp namespace contracts.
pub fn register_mvp_namespace_contracts(
    registry: &mut FunctionRegistry,
) -> Result<(), RegistryError> {
    let evidence = FunctionSpec::new(
        FunctionName::new("evidence.supporting_count")?,
        FunctionSignature::new(vec![FunctionValueType::String], FunctionValueType::Integer)?,
        FunctionDeterminism::Deterministic,
        FunctionCostClass::Low,
    )?
    .with_core_handler(evidence_supporting_count);

    let confidence = FunctionSpec::new(
        FunctionName::new("confidence.clamp")?,
        FunctionSignature::new(vec![FunctionValueType::Float], FunctionValueType::Float)?,
        FunctionDeterminism::Deterministic,
        FunctionCostClass::Low,
    )?
    .with_core_handler(confidence_clamp);

    let temporal = FunctionSpec::new(
        FunctionName::new("temporal.window_overlap_days")?,
        FunctionSignature::new(
            vec![FunctionValueType::String, FunctionValueType::String],
            FunctionValueType::Integer,
        )?,
        FunctionDeterminism::Deterministic,
        FunctionCostClass::Medium,
    )?
    .with_core_handler(temporal_window_overlap_days);

    let cti = FunctionSpec::new(
        FunctionName::new("cti.observable_kind")?,
        FunctionSignature::new(vec![FunctionValueType::String], FunctionValueType::String)?,
        FunctionDeterminism::NonDeterministic,
        FunctionCostClass::Medium,
    )?
    .with_model_adapter_key("cti-adapter")?;

    let fimi = FunctionSpec::new(
        FunctionName::new("fimi.claim_similarity")?,
        FunctionSignature::new(
            vec![FunctionValueType::String, FunctionValueType::String],
            FunctionValueType::Integer,
        )?,
        FunctionDeterminism::NonDeterministic,
        FunctionCostClass::High,
    )?
    .with_model_adapter_key("fimi-adapter")?;

    let coordination = FunctionSpec::new(
        FunctionName::new("coordination.window_score")?,
        FunctionSignature::new(vec![FunctionValueType::Integer], FunctionValueType::Integer)?,
        FunctionDeterminism::Deterministic,
        FunctionCostClass::Medium,
    )?
    .with_core_handler(coordination_window_score);

    let crisis = FunctionSpec::new(
        FunctionName::new("crisis.classification_score")?,
        FunctionSignature::new(vec![FunctionValueType::String], FunctionValueType::Integer)?,
        FunctionDeterminism::NonDeterministic,
        FunctionCostClass::High,
    )?
    .with_model_adapter_key("crisis-adapter")?;

    registry.register(evidence)?;
    registry.register(confidence)?;
    registry.register(temporal)?;
    registry.register(cti)?;
    registry.register(fimi)?;
    registry.register(coordination)?;
    registry.register(crisis)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn function_name_validation_accepts_trimmed_namespaced_values() {
        let name = FunctionName::new(" evidence.supporting_count ")
            .expect("trimmed namespaced value should be accepted");
        assert_eq!(name.as_str(), "evidence.supporting_count");
    }

    #[test]
    fn function_name_validation_rejects_invalid_shapes() {
        assert!(matches!(
            FunctionName::new(""),
            Err(RegistryError::InvalidFunctionName(name)) if name.is_empty()
        ));
        assert!(matches!(
            FunctionName::new("namespace"),
            Err(RegistryError::InvalidFunctionName(name)) if name == "namespace"
        ));
        assert!(matches!(
            FunctionName::new("namespace."),
            Err(RegistryError::InvalidFunctionName(name)) if name == "namespace."
        ));
        assert!(matches!(
            FunctionName::new("namespace.symbol.extra"),
            Err(RegistryError::InvalidFunctionName(name)) if name == "namespace.symbol.extra"
        ));
    }

    #[test]
    fn private_core_handlers_cover_success_and_type_mismatch_paths() {
        assert_eq!(
            evidence_supporting_count(&[FunctionValue::String("abcd".to_owned())]),
            Ok(FunctionValue::Integer(4))
        );
        assert!(matches!(
            evidence_supporting_count(&[FunctionValue::Integer(1)]),
            Err(FunctionRecoverableError { code, .. }) if code == "TYPE_MISMATCH"
        ));

        assert_eq!(
            confidence_clamp(&[FunctionValue::Float(1.5)]),
            Ok(FunctionValue::Float(1.0))
        );
        assert!(matches!(
            confidence_clamp(&[FunctionValue::String("x".to_owned())]),
            Err(FunctionRecoverableError { code, .. }) if code == "TYPE_MISMATCH"
        ));

        assert_eq!(
            temporal_window_overlap_days(&[
                FunctionValue::String("2026-07-07".to_owned()),
                FunctionValue::String("2026-07-07".to_owned()),
            ]),
            Ok(FunctionValue::Integer(1))
        );
        assert_eq!(
            temporal_window_overlap_days(&[
                FunctionValue::String("2026-07-07".to_owned()),
                FunctionValue::String("2026-07-08".to_owned()),
            ]),
            Ok(FunctionValue::Integer(0))
        );
        assert!(matches!(
            temporal_window_overlap_days(&[
                FunctionValue::Integer(1),
                FunctionValue::Integer(2),
            ]),
            Err(FunctionRecoverableError { code, .. }) if code == "TYPE_MISMATCH"
        ));

        assert_eq!(
            coordination_window_score(&[FunctionValue::Integer(130)]),
            Ok(FunctionValue::Integer(100))
        );
        assert!(matches!(
            coordination_window_score(&[FunctionValue::Boolean(true)]),
            Err(FunctionRecoverableError { code, .. }) if code == "TYPE_MISMATCH"
        ));
    }

    #[test]
    fn recoverable_error_and_model_adapter_key_validate_trimmed_inputs() {
        let recoverable = FunctionRecoverableError::new(" MODEL_TIMEOUT ", " retry later ", true)
            .expect("trimmed recoverable error payload should be accepted");
        assert_eq!(recoverable.code, "MODEL_TIMEOUT");
        assert_eq!(recoverable.message, "retry later");

        let spec = FunctionSpec::new(
            FunctionName::new("fimi.claim_similarity").expect("function name should be valid"),
            FunctionSignature::new(
                vec![FunctionValueType::String, FunctionValueType::String],
                FunctionValueType::Integer,
            )
            .expect("signature should be valid"),
            FunctionDeterminism::NonDeterministic,
            FunctionCostClass::High,
        )
        .expect("spec should be valid");

        let with_key = spec
            .clone()
            .with_model_adapter_key(" fimi-adapter ")
            .expect("trimmed model adapter key should be accepted");
        assert_eq!(with_key.model_adapter_key.as_deref(), Some("fimi-adapter"));

        let error = spec
            .with_model_adapter_key(" ")
            .expect_err("empty adapter key should be rejected");
        assert!(matches!(
            error,
            RegistryError::InvalidModelAdapterKey(key) if key.is_empty()
        ));
    }

    #[test]
    fn function_value_type_reports_expected_variant_types() {
        assert_eq!(
            FunctionValue::String("x".to_owned()).value_type(),
            FunctionValueType::String
        );
        assert_eq!(
            FunctionValue::Integer(1).value_type(),
            FunctionValueType::Integer
        );
        assert_eq!(
            FunctionValue::Float(1.0).value_type(),
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
            FunctionValue::List(vec![]).value_type(),
            FunctionValueType::List
        );
        assert_eq!(
            FunctionValue::Object(HashMap::new()).value_type(),
            FunctionValueType::Object
        );
        assert_eq!(FunctionValue::Null.value_type(), FunctionValueType::Null);
    }

    #[test]
    fn recoverable_error_display_and_validation_errors() {
        let error = FunctionRecoverableError::new("CODE_A", "message a", false)
            .expect("recoverable error should be constructible");
        assert_eq!(format!("{error}"), "CODE_A: message a");

        assert!(matches!(
            FunctionRecoverableError::new(" ", "message", true),
            Err(RegistryError::InvalidRecoverableErrorCode(code)) if code.is_empty()
        ));
        assert!(matches!(
            FunctionRecoverableError::new("CODE", " ", true),
            Err(RegistryError::InvalidRecoverableErrorMessage(message)) if message.is_empty()
        ));
    }

    #[test]
    fn execution_policy_defaults_and_timeout_validation() {
        let default_policy = FunctionExecutionPolicy::default_core();
        assert_eq!(default_policy.timeout_ms, 1_000);
        assert_eq!(default_policy.cache_policy, FunctionCachePolicy::NoCache);
        assert_eq!(
            default_policy.permissions,
            vec![FunctionPermission::ReadGraph]
        );

        let custom = FunctionExecutionPolicy::new(
            250,
            FunctionCachePolicy::TtlSeconds(30),
            vec![
                FunctionPermission::ReadGraph,
                FunctionPermission::ModelAdapter,
            ],
        )
        .expect("non-zero timeout should be accepted");
        assert_eq!(custom.timeout_ms, 250);
        assert_eq!(custom.cache_policy, FunctionCachePolicy::TtlSeconds(30));
        assert_eq!(custom.permissions.len(), 2);

        assert!(matches!(
            FunctionExecutionPolicy::new(0, FunctionCachePolicy::NoCache, vec![]),
            Err(RegistryError::InvalidTimeoutMs(0))
        ));
    }

    #[test]
    fn function_signature_new_and_validate_arguments_cover_all_error_paths() {
        assert!(matches!(
            FunctionSignature::new(vec![], FunctionValueType::Integer),
            Err(RegistryError::MissingInputTypes)
        ));

        let signature = FunctionSignature::new(
            vec![FunctionValueType::String, FunctionValueType::Integer],
            FunctionValueType::Boolean,
        )
        .expect("signature should be valid");

        signature
            .validate_arguments(&[
                FunctionValue::String("abc".to_owned()),
                FunctionValue::Integer(7),
            ])
            .expect("matching argument shape should validate");

        assert!(matches!(
            signature.validate_arguments(&[FunctionValue::String("abc".to_owned())]),
            Err(RegistryError::ArgumentArityMismatch {
                expected: 2,
                actual: 1,
            })
        ));

        assert!(matches!(
            signature.validate_arguments(&[
                FunctionValue::String("abc".to_owned()),
                FunctionValue::Boolean(true),
            ]),
            Err(RegistryError::ArgumentTypeMismatch {
                index: 1,
                expected: FunctionValueType::Integer,
                actual: FunctionValueType::Boolean,
            })
        ));
    }

    #[test]
    fn function_registry_register_and_get_enforce_uniqueness() {
        let mut registry = FunctionRegistry::new();
        let spec = FunctionSpec::new(
            FunctionName::new("evidence.supporting_count").expect("name should be valid"),
            FunctionSignature::new(vec![FunctionValueType::String], FunctionValueType::Integer)
                .expect("signature should be valid"),
            FunctionDeterminism::Deterministic,
            FunctionCostClass::Low,
        )
        .expect("spec should be valid")
        .with_core_handler(evidence_supporting_count);

        registry
            .register(spec)
            .expect("first registration should succeed");
        assert!(registry.get("evidence.supporting_count").is_some());
        assert!(registry.get("missing.function").is_none());

        let duplicate = FunctionSpec::new(
            FunctionName::new("evidence.supporting_count").expect("name should be valid"),
            FunctionSignature::new(vec![FunctionValueType::String], FunctionValueType::Integer)
                .expect("signature should be valid"),
            FunctionDeterminism::Deterministic,
            FunctionCostClass::Low,
        )
        .expect("spec should be valid")
        .with_core_handler(evidence_supporting_count);

        assert!(matches!(
            registry.register(duplicate),
            Err(RegistryError::FunctionAlreadyRegistered(name)) if name == "evidence.supporting_count"
        ));
    }

    #[test]
    fn invoke_reports_not_found_and_missing_core_handler() {
        let registry = FunctionRegistry::new();
        assert!(matches!(
            registry.invoke("unknown.fn", &[], None),
            Err(RegistryError::FunctionNotFound(name)) if name == "unknown.fn"
        ));

        let mut registry = FunctionRegistry::new();
        let spec = FunctionSpec::new(
            FunctionName::new("evidence.no_handler").expect("name should be valid"),
            FunctionSignature::new(vec![FunctionValueType::String], FunctionValueType::Integer)
                .expect("signature should be valid"),
            FunctionDeterminism::Deterministic,
            FunctionCostClass::Low,
        )
        .expect("spec should be valid");
        registry
            .register(spec)
            .expect("registration should succeed");

        assert!(matches!(
            registry.invoke("evidence.no_handler", &[FunctionValue::String("x".to_owned())], None),
            Err(RegistryError::MissingCoreHandler(name)) if name == "evidence.no_handler"
        ));
    }

    #[test]
    fn invoke_model_adapter_path_covers_missing_mismatch_and_recoverable_error() {
        struct FakeAdapter {
            key: String,
            result: Result<FunctionValue, FunctionRecoverableError>,
            calls: RefCell<Vec<(String, usize, u64)>>,
        }

        impl ModelFunctionAdapter for FakeAdapter {
            fn key(&self) -> &str {
                &self.key
            }

            fn invoke(
                &self,
                function_name: &str,
                args: &[FunctionValue],
                policy: &FunctionExecutionPolicy,
            ) -> Result<FunctionValue, FunctionRecoverableError> {
                self.calls.borrow_mut().push((
                    function_name.to_owned(),
                    args.len(),
                    policy.timeout_ms,
                ));
                self.result.clone()
            }
        }

        let mut registry = FunctionRegistry::new();
        let spec = FunctionSpec::new(
            FunctionName::new("cti.observable_kind").expect("name should be valid"),
            FunctionSignature::new(vec![FunctionValueType::String], FunctionValueType::String)
                .expect("signature should be valid"),
            FunctionDeterminism::NonDeterministic,
            FunctionCostClass::Medium,
        )
        .expect("spec should be valid")
        .with_execution_policy(
            FunctionExecutionPolicy::new(
                200,
                FunctionCachePolicy::NoCache,
                vec![FunctionPermission::ModelAdapter],
            )
            .expect("policy should be valid"),
        )
        .with_model_adapter_key("cti-adapter")
        .expect("adapter key should be valid");
        registry
            .register(spec)
            .expect("registration should succeed");

        let args = vec![FunctionValue::String("ipv4-addr".to_owned())];

        assert!(matches!(
            registry.invoke("cti.observable_kind", &args, None),
            Err(RegistryError::MissingModelAdapter(name)) if name == "cti.observable_kind"
        ));

        let mismatch_adapter = FakeAdapter {
            key: "other-adapter".to_owned(),
            result: Ok(FunctionValue::String("ignored".to_owned())),
            calls: RefCell::new(Vec::new()),
        };
        assert!(matches!(
            registry.invoke("cti.observable_kind", &args, Some(&mismatch_adapter)),
            Err(RegistryError::ModelAdapterMismatch {
                function_name,
                expected_key,
                provided_key,
            }) if function_name == "cti.observable_kind" && expected_key == "cti-adapter" && provided_key == "other-adapter"
        ));

        let recoverable = FunctionRecoverableError::new("MODEL_BUSY", "retry", true)
            .expect("recoverable payload should be valid");
        let error_adapter = FakeAdapter {
            key: "cti-adapter".to_owned(),
            result: Err(recoverable.clone()),
            calls: RefCell::new(Vec::new()),
        };
        assert!(matches!(
            registry.invoke("cti.observable_kind", &args, Some(&error_adapter)),
            Err(RegistryError::FunctionExecutionRecoverable(err)) if err == recoverable
        ));
        assert_eq!(error_adapter.calls.borrow().len(), 1);
        assert_eq!(error_adapter.calls.borrow()[0].0, "cti.observable_kind");
        assert_eq!(error_adapter.calls.borrow()[0].1, 1);
        assert_eq!(error_adapter.calls.borrow()[0].2, 200);
    }

    #[test]
    fn register_mvp_namespace_contracts_registers_expected_specs_and_invokes_core_handlers() {
        let mut registry = FunctionRegistry::new();
        register_mvp_namespace_contracts(&mut registry)
            .expect("mvp namespace contracts should register");

        assert!(registry.get("evidence.supporting_count").is_some());
        assert!(registry.get("confidence.clamp").is_some());
        assert!(registry.get("temporal.window_overlap_days").is_some());
        assert!(registry.get("coordination.window_score").is_some());
        assert_eq!(
            registry
                .get("cti.observable_kind")
                .expect("cti function should be present")
                .model_adapter_key
                .as_deref(),
            Some("cti-adapter")
        );

        assert_eq!(
            registry
                .invoke(
                    "evidence.supporting_count",
                    &[FunctionValue::String("abcd".to_owned())],
                    None,
                )
                .expect("core evidence handler should run"),
            FunctionValue::Integer(4)
        );
        assert_eq!(
            registry
                .invoke("confidence.clamp", &[FunctionValue::Float(3.0)], None)
                .expect("core confidence handler should run"),
            FunctionValue::Float(1.0)
        );

        assert!(matches!(
            register_mvp_namespace_contracts(&mut registry),
            Err(RegistryError::FunctionAlreadyRegistered(name)) if name == "evidence.supporting_count"
        ));
    }
}
