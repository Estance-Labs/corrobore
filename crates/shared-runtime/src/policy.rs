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
use crate::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Runtime policy.
pub struct RuntimePolicy {
    /// Allowed request modes.
    pub allowed_request_modes: Vec<CypherRequestMode>,
    /// Max query length.
    pub max_query_length: usize,
    /// Max parameter count.
    pub max_parameter_count: usize,
    /// Mutation permissions.
    pub mutation_permissions: bool,
    /// Audit policy references.
    pub audit_policy_references: Vec<String>,
}

impl RuntimePolicy {
    /// Strict default.
    pub fn strict_default() -> Self {
        Self {
            // Allowed request modes.
            allowed_request_modes: vec![
                CypherRequestMode::ReadOnly,
                CypherRequestMode::Mutation,
                CypherRequestMode::Explain,
                CypherRequestMode::ValidateOnly,
            ],
            // Max query length.
            max_query_length: 8_192,
            // Max parameter count.
            max_parameter_count: 128,
            // Mutation permissions.
            mutation_permissions: true,
            // Audit policy references.
            audit_policy_references: vec!["audit-policy--default".to_owned()],
        }
    }
}
