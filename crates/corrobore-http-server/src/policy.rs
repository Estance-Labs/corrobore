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

//! Authorization of graph actions with Cedar.
//!
//! A policy file names groups of actors, the protected branches, and rules
//! that allow actions to a group, to listed actors or to any authenticated
//! actor, optionally only on protected or unprotected branches. It compiles
//! to one Cedar `permit` per rule and action, validated against a fixed
//! schema, so a decision is Cedar's decision. Nothing is allowed that no rule
//! allows; rules never forbid, so their order does not matter.
//!
//! A request names an action, the branch it acts on, and for branch
//! creation, merge and deletion the branch it targets. A decision without a
//! branch is "graph-wide": only rules that hold on every branch allow it.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    str::FromStr,
};

use cedar_policy::{
    Authorizer, Context, Decision, Entities, Entity, EntityId, EntityTypeName, EntityUid, Policy,
    PolicyId, PolicySet, Request, RestrictedExpression, Schema, ValidationMode, Validator,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Version of the policy file format.
pub const POLICY_FORMAT_VERSION: u32 = 1;
/// The single graph a Corrobore server serves.
const GRAPH_RESOURCE: &str = "graph";

/// What a request does to the graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyAction {
    /// Queries, history, schema, search, records, content reads.
    Read,
    /// Exports of graph data (STIX, Parquet, snapshots).
    Export,
    /// Writes: Cypher writes, edits, reverts, imports, content uploads.
    Change,
    /// Recording a review of a branch head.
    Review,
    /// Creating a branch: `branch` is the source, `target_branch` the new one.
    BranchCreate,
    /// Deleting `target_branch`.
    BranchDelete,
    /// Merging `branch` into `target_branch`.
    BranchMerge,
    /// Content-preserving maintenance such as index rebuilds.
    Maintain,
    /// Cleanup that permanently removes history.
    Reclaim,
    /// Running a stored query (its own `read` or `change` is checked too).
    InvokeQuery,
    /// Administration: operation status, provider status, OpenCTI authority.
    Admin,
}

impl PolicyAction {
    pub const ALL: [Self; 11] = [
        Self::Read,
        Self::Export,
        Self::Change,
        Self::Review,
        Self::BranchCreate,
        Self::BranchDelete,
        Self::BranchMerge,
        Self::Maintain,
        Self::Reclaim,
        Self::InvokeQuery,
        Self::Admin,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Export => "export",
            Self::Change => "change",
            Self::Review => "review",
            Self::BranchCreate => "branch_create",
            Self::BranchDelete => "branch_delete",
            Self::BranchMerge => "branch_merge",
            Self::Maintain => "maintain",
            Self::Reclaim => "reclaim",
            Self::InvokeQuery => "invoke_query",
            Self::Admin => "admin",
        }
    }

    /// Whether the action is judged on a source branch (`branch_scope`).
    pub fn takes_branch(self) -> bool {
        matches!(
            self,
            Self::Read
                | Self::Export
                | Self::Change
                | Self::Review
                | Self::BranchCreate
                | Self::BranchMerge
                | Self::Maintain
        )
    }

    /// Whether the action is judged on a target branch (`target_branch_scope`).
    pub fn takes_target(self) -> bool {
        matches!(
            self,
            Self::BranchCreate | Self::BranchMerge | Self::BranchDelete
        )
    }
}

impl FromStr for PolicyAction {
    type Err = PolicyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|action| action.as_str() == value)
            .ok_or_else(|| PolicyError::Invalid(format!("unknown action {value:?}")))
    }
}

/// Which branches a rule covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchScope {
    Any,
    Protected,
    Unprotected,
}

/// Who a rule allows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ActorSelector {
    /// `actors: any`: every authenticated actor.
    Keyword(AnyActor),
    /// `actors: { group: readers }`.
    Group { group: String },
    /// `actors: { ids: [act-alice, principal:bob] }`.
    Ids { ids: Vec<String> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnyActor {
    Any,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllowRule {
    pub actors: ActorSelector,
    pub actions: Vec<PolicyAction>,
    #[serde(default)]
    pub branch_scope: Option<BranchScope>,
    #[serde(default)]
    pub target_branch_scope: Option<BranchScope>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyRule {
    pub id: String,
    pub allow: AllowRule,
}

/// The policy file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyDocument {
    pub version: u32,
    #[serde(default)]
    pub groups: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub protected_branches: Vec<String>,
    #[serde(default)]
    pub rules: Vec<PolicyRule>,
}

/// One decision to make.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyRequest {
    pub action: PolicyAction,
    pub branch: Option<String>,
    pub target_branch: Option<String>,
}

impl PolicyRequest {
    pub fn graph_wide(action: PolicyAction) -> Self {
        Self {
            action,
            branch: None,
            target_branch: None,
        }
    }

    pub fn on(action: PolicyAction, branch: impl Into<String>) -> Self {
        Self {
            action,
            branch: Some(branch.into()),
            target_branch: None,
        }
    }

    pub fn between(
        action: PolicyAction,
        branch: Option<String>,
        target_branch: impl Into<String>,
    ) -> Self {
        Self {
            action,
            branch,
            target_branch: Some(target_branch.into()),
        }
    }
}

/// The outcome of one request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PolicyDecision {
    pub allowed: bool,
    /// Rules that allowed it; for `policy explain`, never sent to clients.
    pub rules: Vec<String>,
}

/// What an actor may do, as `GET /v1/graph/access` reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AccessReport {
    pub actor_id: Option<String>,
    pub graph_wide: Vec<PolicyAction>,
    pub branch: Option<String>,
    pub target_branch: Option<String>,
    pub actions: Vec<PolicyAction>,
}

#[derive(Debug, Error)]
pub enum PolicyError {
    #[error("policy file is not valid YAML: {0}")]
    Syntax(String),
    #[error("policy is invalid: {0}")]
    Invalid(String),
    #[error("policy does not compile to valid Cedar: {0}")]
    Cedar(String),
}

/// A compiled, validated policy.
#[derive(Debug)]
pub struct PolicyEngine {
    document: PolicyDocument,
    policies: PolicySet,
    entities: Entities,
    schema: Schema,
    protected: BTreeSet<String>,
    rule_of: HashMap<String, String>,
}

impl PolicyEngine {
    /// Parse and compile a YAML (or JSON) policy file.
    pub fn from_yaml(text: &str) -> Result<Self, PolicyError> {
        let document: PolicyDocument =
            yaml_serde::from_str(text).map_err(|error| PolicyError::Syntax(error.to_string()))?;
        Self::compile(document)
    }

    pub fn compile(document: PolicyDocument) -> Result<Self, PolicyError> {
        validate_document(&document)?;
        let (schema, _) = Schema::from_cedarschema_str(SCHEMA)
            .map_err(|error| PolicyError::Cedar(error.to_string()))?;
        let mut policies = Vec::new();
        let mut rule_of = HashMap::new();
        for rule in &document.rules {
            for action in &rule.allow.actions {
                let id = format!("{}:{}", rule.id, action.as_str());
                let policy = Policy::parse(Some(PolicyId::new(&id)), policy_source(rule, *action))
                    .map_err(|error| PolicyError::Cedar(error.to_string()))?;
                rule_of.insert(id, rule.id.clone());
                policies.push(policy);
            }
        }
        let policies = PolicySet::from_policies(policies)
            .map_err(|error| PolicyError::Cedar(error.to_string()))?;
        let validation = Validator::new(schema.clone()).validate(&policies, ValidationMode::Strict);
        if !validation.validation_passed() {
            let errors: Vec<String> = validation
                .validation_errors()
                .map(ToString::to_string)
                .collect();
            return Err(PolicyError::Cedar(errors.join("; ")));
        }
        let entities = compile_entities(&document, &schema)?;
        let protected = document.protected_branches.iter().cloned().collect();
        Ok(Self {
            document,
            policies,
            entities,
            schema,
            protected,
            rule_of,
        })
    }

    pub fn document(&self) -> &PolicyDocument {
        &self.document
    }

    pub fn is_protected(&self, branch: &str) -> bool {
        self.protected.contains(branch)
    }

    /// Decide `request` for `actor`. A malformed branch name is denied.
    pub fn authorize(&self, actor: &str, request: &PolicyRequest) -> PolicyDecision {
        let denied = PolicyDecision {
            allowed: false,
            rules: Vec::new(),
        };
        if request
            .branch
            .as_deref()
            .is_some_and(|b| !valid_branch_name(b))
            || request
                .target_branch
                .as_deref()
                .is_some_and(|b| !valid_branch_name(b))
        {
            return denied;
        }
        let Ok(principal) = uid("Actor", actor) else {
            return denied;
        };
        let Ok(action) = uid("Action", request.action.as_str()) else {
            return denied;
        };
        let Ok(resource) = uid("Graph", GRAPH_RESOURCE) else {
            return denied;
        };
        let branch = request.branch.clone().unwrap_or_default();
        let target = request.target_branch.clone().unwrap_or_default();
        let context = Context::from_pairs([
            (
                "has_branch".to_owned(),
                RestrictedExpression::new_bool(request.branch.is_some()),
            ),
            (
                "branch_is_protected".to_owned(),
                RestrictedExpression::new_bool(self.is_protected(&branch)),
            ),
            (
                "has_target_branch".to_owned(),
                RestrictedExpression::new_bool(request.target_branch.is_some()),
            ),
            (
                "target_branch_is_protected".to_owned(),
                RestrictedExpression::new_bool(self.is_protected(&target)),
            ),
        ]);
        let Ok(context) = context else {
            return denied;
        };
        let Ok(request) = Request::new(principal, action, resource, context, Some(&self.schema))
        else {
            return denied;
        };
        let response = Authorizer::new().is_authorized(&request, &self.policies, &self.entities);
        let mut rules: Vec<String> = response
            .diagnostics()
            .reason()
            .filter_map(|id| self.rule_of.get(id.to_string().as_str()).cloned())
            .collect();
        rules.sort();
        rules.dedup();
        PolicyDecision {
            allowed: response.decision() == Decision::Allow,
            rules,
        }
    }

    /// Every action `actor` may take graph-wide, on `branch`, and from
    /// `branch` into `target_branch`, each shaped as the route performing it.
    pub fn access(
        &self,
        actor: &str,
        branch: Option<&str>,
        target_branch: Option<&str>,
    ) -> AccessReport {
        let allowed = |request: PolicyRequest| self.authorize(actor, &request).allowed;
        let graph_wide = PolicyAction::ALL
            .into_iter()
            .filter(|action| allowed(PolicyRequest::graph_wide(*action)))
            .collect();
        let mut actions = Vec::new();
        if let Some(branch) = branch {
            for action in PolicyAction::ALL {
                let request = match action {
                    PolicyAction::BranchDelete => {
                        PolicyRequest::between(action, None, branch.to_owned())
                    }
                    PolicyAction::BranchCreate | PolicyAction::BranchMerge => match target_branch {
                        Some(target) => PolicyRequest::between(
                            action,
                            Some(branch.to_owned()),
                            target.to_owned(),
                        ),
                        None => continue,
                    },
                    action if action.takes_branch() => PolicyRequest::on(action, branch),
                    _ => continue,
                };
                if allowed(request) {
                    actions.push(action);
                }
            }
        }
        AccessReport {
            actor_id: Some(actor.to_owned()),
            graph_wide,
            branch: branch.map(str::to_owned),
            target_branch: target_branch.map(str::to_owned),
            actions,
        }
    }
}

/// Branch names a policy can judge: non-empty, at most 255 bytes, without
/// surrounding whitespace or control characters.
pub fn valid_branch_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name.trim() == name
        && !name.chars().any(char::is_control)
}

fn validate_document(document: &PolicyDocument) -> Result<(), PolicyError> {
    let invalid = |message: String| Err(PolicyError::Invalid(message));
    if document.version != POLICY_FORMAT_VERSION {
        return invalid(format!(
            "version {} is not supported; use version {POLICY_FORMAT_VERSION}",
            document.version
        ));
    }
    for branch in &document.protected_branches {
        if !valid_branch_name(branch) {
            return invalid(format!("protected branch {branch:?} is not a valid name"));
        }
    }
    for (group, members) in &document.groups {
        if !valid_name(group) {
            return invalid(format!("group {group:?} is not a valid name"));
        }
        for member in members {
            if !crate::config::valid_actor_id(member) {
                return invalid(format!("group {group} lists an invalid actor {member:?}"));
            }
        }
    }
    let mut ids = HashSet::new();
    for rule in &document.rules {
        if !valid_name(&rule.id) || !ids.insert(rule.id.as_str()) {
            return invalid(format!("rule id {:?} is invalid or repeated", rule.id));
        }
        let allow = &rule.allow;
        if allow.actions.is_empty() {
            return invalid(format!("rule {} allows no action", rule.id));
        }
        match &allow.actors {
            ActorSelector::Group { group } if !document.groups.contains_key(group) => {
                return invalid(format!("rule {} names an unknown group {group}", rule.id));
            }
            ActorSelector::Ids { ids }
                if ids.is_empty() || ids.iter().any(|id| !crate::config::valid_actor_id(id)) =>
            {
                return invalid(format!("rule {} lists an invalid actor", rule.id));
            }
            _ => {}
        }
        if allow.branch_scope.is_some() && allow.target_branch_scope.is_some() {
            return invalid(format!(
                "rule {} sets both branch_scope and target_branch_scope",
                rule.id
            ));
        }
        if allow.branch_scope.is_some()
            && let Some(action) = allow.actions.iter().find(|a| !a.takes_branch())
        {
            return invalid(format!(
                "rule {}: {} takes no branch_scope",
                rule.id,
                action.as_str()
            ));
        }
        if allow.target_branch_scope.is_some()
            && let Some(action) = allow.actions.iter().find(|a| !a.takes_target())
        {
            return invalid(format!(
                "rule {}: {} takes no target_branch_scope",
                rule.id,
                action.as_str()
            ));
        }
    }
    Ok(())
}

/// Group and rule ids: 1-64 characters of letters, digits and `._-`.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn policy_source(rule: &PolicyRule, action: PolicyAction) -> String {
    let mut conditions = Vec::new();
    let principal = match &rule.allow.actors {
        ActorSelector::Keyword(AnyActor::Any) => "principal".to_owned(),
        ActorSelector::Group { group } => {
            format!("principal in Corrobore::Group::{}", literal(group))
        }
        ActorSelector::Ids { ids } => {
            let list = ids
                .iter()
                .map(|id| format!("Corrobore::Actor::{}", literal(id)))
                .collect::<Vec<_>>()
                .join(", ");
            conditions.push(format!("[{list}].contains(principal)"));
            "principal".to_owned()
        }
    };
    if let Some(scope) = rule.allow.branch_scope {
        conditions.push(scope_condition(scope, "has_branch", "branch_is_protected"));
    }
    if let Some(scope) = rule.allow.target_branch_scope {
        conditions.push(scope_condition(
            scope,
            "has_target_branch",
            "target_branch_is_protected",
        ));
    }
    let when = if conditions.is_empty() {
        String::new()
    } else {
        format!(" when {{ {} }}", conditions.join(" && "))
    };
    format!(
        "permit ({principal}, action == Corrobore::Action::{action}, resource == Corrobore::Graph::{graph}){when};",
        action = literal(action.as_str()),
        graph = literal(GRAPH_RESOURCE),
    )
}

fn scope_condition(scope: BranchScope, has: &str, protected: &str) -> String {
    match scope {
        BranchScope::Any => "true".to_owned(),
        BranchScope::Protected => format!("context.{has} && context.{protected}"),
        BranchScope::Unprotected => format!("context.{has} && !context.{protected}"),
    }
}

fn compile_entities(document: &PolicyDocument, schema: &Schema) -> Result<Entities, PolicyError> {
    let cedar = |error: &dyn std::fmt::Display| PolicyError::Cedar(error.to_string());
    let mut entities = Vec::new();
    let mut memberships: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (group, members) in &document.groups {
        entities.push(Entity::new_no_attrs(
            uid("Group", group).map_err(|e| cedar(&e))?,
            HashSet::new(),
        ));
        for member in members {
            memberships.entry(member).or_default().insert(group);
        }
    }
    for (actor, groups) in memberships {
        let parents = groups
            .into_iter()
            .map(|group| uid("Group", group))
            .collect::<Result<HashSet<_>, _>>()
            .map_err(|e| cedar(&e))?;
        entities.push(Entity::new_no_attrs(
            uid("Actor", actor).map_err(|e| cedar(&e))?,
            parents,
        ));
    }
    entities.push(Entity::new_no_attrs(
        uid("Graph", GRAPH_RESOURCE).map_err(|e| cedar(&e))?,
        HashSet::new(),
    ));
    Entities::from_entities(entities, Some(schema)).map_err(|e| cedar(&e))
}

fn uid(kind: &str, id: &str) -> Result<EntityUid, String> {
    let kind =
        EntityTypeName::from_str(&format!("Corrobore::{kind}")).map_err(|e| e.to_string())?;
    let id = EntityId::from_str(id).map_err(|e| e.to_string())?;
    Ok(EntityUid::from_type_name_and_id(kind, id))
}

fn literal(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

const SCHEMA: &str = r#"
namespace Corrobore {
    type RequestContext = {
        has_branch: Bool,
        branch_is_protected: Bool,
        has_target_branch: Bool,
        target_branch_is_protected: Bool,
    };
    entity Group;
    entity Actor in [Group];
    entity Graph;
    action "read", "export", "change", "review", "branch_create", "branch_delete",
        "branch_merge", "maintain", "reclaim", "invoke_query", "admin"
        appliesTo { principal: Actor, resource: Graph, context: RequestContext };
}
"#;

/// A `policy test` case file.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyTests {
    pub version: u32,
    #[serde(default)]
    pub cases: Vec<PolicyTestCase>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyTestCase {
    pub id: String,
    pub actor: String,
    pub action: PolicyAction,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub target_branch: Option<String>,
    pub expect: Expectation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Expectation {
    Allow,
    Deny,
}

/// One failed case of a test run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FailedCase {
    pub id: String,
    pub expected: Expectation,
}

impl PolicyEngine {
    /// Run a test file; the failed cases, empty when all pass.
    pub fn run_tests(&self, tests: &PolicyTests) -> Result<Vec<FailedCase>, PolicyError> {
        if tests.version != POLICY_FORMAT_VERSION {
            return Err(PolicyError::Invalid(format!(
                "test file version {} is not supported",
                tests.version
            )));
        }
        Ok(tests
            .cases
            .iter()
            .filter_map(|case| {
                let request = PolicyRequest {
                    action: case.action,
                    branch: case.branch.clone(),
                    target_branch: case.target_branch.clone(),
                };
                let allowed = self.authorize(&case.actor, &request).allowed;
                let expected = case.expect == Expectation::Allow;
                (allowed != expected).then(|| FailedCase {
                    id: case.id.clone(),
                    expected: case.expect,
                })
            })
            .collect())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{PolicyAction, PolicyEngine, PolicyRequest};

    const POLICY: &str = "
version: 1
groups:
  analysts: [act-ann]
protected_branches: [main, release]
rules:
  - id: analysts-read
    allow: { actors: { group: analysts }, actions: [read], branch_scope: any }
  - id: analysts-write-drafts
    allow: { actors: { group: analysts }, actions: [change], branch_scope: unprotected }
  - id: bob-merges-into-protected
    allow: { actors: { ids: [act-bob] }, actions: [branch_merge], target_branch_scope: protected }
  - id: everyone-invokes
    allow: { actors: any, actions: [invoke_query] }
";

    fn engine() -> PolicyEngine {
        PolicyEngine::from_yaml(POLICY).unwrap()
    }

    #[test]
    fn scopes_follow_protected_branches() {
        let engine = engine();
        let allowed =
            |actor: &str, request: PolicyRequest| engine.authorize(actor, &request).allowed;
        assert!(allowed(
            "act-ann",
            PolicyRequest::on(PolicyAction::Read, "main")
        ));
        assert!(allowed(
            "act-ann",
            PolicyRequest::graph_wide(PolicyAction::Read)
        ));
        assert!(allowed(
            "act-ann",
            PolicyRequest::on(PolicyAction::Change, "drafts")
        ));
        assert!(!allowed(
            "act-ann",
            PolicyRequest::on(PolicyAction::Change, "release")
        ));
        // A scoped rule never allows the graph-wide decision.
        assert!(!allowed(
            "act-ann",
            PolicyRequest::graph_wide(PolicyAction::Change)
        ));
        assert!(!allowed(
            "act-eve",
            PolicyRequest::on(PolicyAction::Read, "drafts")
        ));
        let merge = |target: &str| {
            PolicyRequest::between(PolicyAction::BranchMerge, Some("drafts".to_owned()), target)
        };
        assert!(allowed("act-bob", merge("main")));
        assert!(!allowed("act-bob", merge("feature")));
        assert!(allowed(
            "act-eve",
            PolicyRequest::graph_wide(PolicyAction::InvokeQuery)
        ));
    }

    #[test]
    fn explain_names_the_allowing_rule() {
        let decision = engine().authorize("act-ann", &PolicyRequest::on(PolicyAction::Change, "x"));
        assert!(decision.allowed);
        assert_eq!(decision.rules, ["analysts-write-drafts"]);
    }

    #[test]
    fn malformed_branch_names_are_denied() {
        let engine = engine();
        for name in ["", " main", "a\nb"] {
            assert!(
                !engine
                    .authorize("act-ann", &PolicyRequest::on(PolicyAction::Read, name))
                    .allowed
            );
        }
    }

    #[test]
    fn access_lists_actions_per_branch() {
        let report = engine().access("act-ann", Some("drafts"), None);
        assert_eq!(
            report.graph_wide,
            [PolicyAction::Read, PolicyAction::InvokeQuery]
        );
        assert_eq!(report.actions, [PolicyAction::Read, PolicyAction::Change]);
    }

    #[test]
    fn invalid_documents_are_refused() {
        for (text, needle) in [
            ("version: 3\n", "version"),
            (
                "version: 1\nrules: [{id: r, allow: {actors: {group: g}, actions: [read]}}]\n",
                "unknown group",
            ),
            (
                "version: 1\nrules: [{id: r, allow: {actors: any, actions: []}}]\n",
                "no action",
            ),
            (
                "version: 1\nrules: [{id: r, allow: {actors: any, actions: [admin], branch_scope: any}}]\n",
                "takes no branch_scope",
            ),
            (
                "version: 1\nrules: [{id: r, allow: {actors: any, actions: [read], target_branch_scope: any}}]\n",
                "takes no target_branch_scope",
            ),
            (
                "version: 1\nrules: [{id: r, allow: {actors: any, actions: [branch_merge], branch_scope: any, target_branch_scope: any}}]\n",
                "both",
            ),
            (
                "version: 1\nrules: [{id: r, allow: {actors: any, actions: [read]}}, {id: r, allow: {actors: any, actions: [read]}}]\n",
                "repeated",
            ),
            ("version: 1\ngroups: {g: ['bad actor']}\n", "invalid actor"),
            ("version: 1\nsurprise: true\n", "surprise"),
        ] {
            let error = PolicyEngine::from_yaml(text).unwrap_err().to_string();
            assert!(error.contains(needle), "{text}: {error}");
        }
    }
}
