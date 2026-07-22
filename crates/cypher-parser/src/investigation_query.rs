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
use serde::Serialize;
use thiserror::Error;

/// Parsed declarative investigation statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvestigationQuery {
    /// Investigation operation requested by the caller.
    pub intent: InvestigationIntent,
    /// Typed graph entity that anchors the investigation.
    pub target: InvestigationTarget,
    /// Optional temporal snapshot requested by `AT TIME`.
    pub at_time: Option<InvestigationTimestamp>,
    /// Hard evidence contracts declared by `REQUIRE`.
    pub requirements: Vec<Requirement>,
    /// Explicitly enabled or disabled investigation behaviors.
    pub allowances: Vec<Allowance>,
    /// Optional bounded resources declared by `BUDGET`.
    pub budget: Option<InvestigationBudget>,
    /// Deterministically ordered response fields declared by `RETURN`.
    pub returns: Vec<ReturnProjection>,
}

impl InvestigationQuery {
    /// Serializes the normalized AST using canonical clause and contract order.
    pub fn to_canonical_string(&self) -> String {
        let mut clauses = vec![format!(
            "INVESTIGATE {} OF {}(\"{}\")",
            self.intent.canonical_name(),
            self.target.kind.canonical_name(),
            self.target.identifier
        )];
        if let Some(at_time) = &self.at_time {
            clauses.push(format!("AT TIME {}", at_time.as_str()));
        }
        if !self.requirements.is_empty() {
            clauses.push(format!(
                "REQUIRE {}",
                self.requirements
                    .iter()
                    .map(Requirement::canonical_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !self.allowances.is_empty() {
            clauses.push(format!(
                "ALLOW {}",
                self.allowances
                    .iter()
                    .map(Allowance::canonical_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(budget) = self.budget {
            let mut contracts = Vec::new();
            if let Some(memory_bytes) = budget.memory_bytes {
                contracts.push(format!("memory = {memory_bytes} B"));
            }
            if let Some(latency_millis) = budget.latency_millis {
                contracts.push(format!("latency = {latency_millis} ms"));
            }
            if let Some(external_retrievals) = budget.external_retrievals {
                contracts.push(format!("external_retrievals = {external_retrievals}"));
            }
            clauses.push(format!("BUDGET {}", contracts.join(", ")));
        }
        clauses.push(format!(
            "RETURN {}",
            self.returns
                .iter()
                .map(|projection| projection.canonical_name())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        clauses.join(" ")
    }
}

/// Supported investigation operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvestigationIntent {
    /// Determine attribution for the target entity.
    Attribution,
}

impl InvestigationIntent {
    fn canonical_name(self) -> &'static str {
        match self {
            Self::Attribution => "attribution",
        }
    }
}

/// Supported investigation target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvestigationTarget {
    /// Typed target category.
    pub kind: InvestigationTargetKind,
    /// Stable target identifier.
    pub identifier: String,
}

/// Target categories accepted by the declarative layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvestigationTargetKind {
    /// Campaign entity.
    Campaign,
    /// Actor entity.
    Actor,
    /// Narrative entity.
    Narrative,
    /// Indicator entity.
    Indicator,
}

impl InvestigationTargetKind {
    fn canonical_name(self) -> &'static str {
        match self {
            Self::Campaign => "Campaign",
            Self::Actor => "Actor",
            Self::Narrative => "Narrative",
            Self::Indicator => "Indicator",
        }
    }
}

/// Validated date or timestamp used for a temporal investigation snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvestigationTimestamp(String);

impl InvestigationTimestamp {
    /// Returns the validated source representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Fixed-point threshold normalized to one million parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct NormalizedThreshold(u32);

impl NormalizedThreshold {
    /// Builds a threshold in the inclusive range from zero to one million.
    pub fn from_parts_per_million(parts_per_million: u32) -> Result<Self, InvestigationParseError> {
        if parts_per_million > 1_000_000 {
            return Err(invalid_value(
                "threshold",
                "threshold must be between 0 and 1 inclusive",
            ));
        }
        Ok(Self(parts_per_million))
    }

    /// Returns the normalized fixed-point value.
    #[must_use]
    pub fn parts_per_million(self) -> u32 {
        self.0
    }
}

/// Hard evidence-quality contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requirement {
    /// Minimum number of mutually independent sources.
    IndependentSourcesAtLeast(u32),
    /// Minimum accepted source reliability.
    SourceReliabilityAtLeast(NormalizedThreshold),
    /// Minimum accepted evidence completeness.
    EvidenceCompletenessAtLeast(NormalizedThreshold),
}

impl Requirement {
    fn canonical_string(&self) -> String {
        match self {
            Self::IndependentSourcesAtLeast(value) => {
                format!("independent_sources >= {value}")
            }
            Self::SourceReliabilityAtLeast(value) => {
                format!("source_reliability >= {}", canonical_threshold(*value))
            }
            Self::EvidenceCompletenessAtLeast(value) => {
                format!("evidence_completeness >= {}", canonical_threshold(*value))
            }
        }
    }
}

/// Explicitly controlled investigation behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Allowance {
    /// Whether hypotheses may be included.
    Hypotheses(bool),
    /// Whether contradictory evidence may be retained.
    ContradictoryEvidence(bool),
}

impl Allowance {
    fn canonical_string(&self) -> String {
        match self {
            Self::Hypotheses(value) => format!("hypotheses = {value}"),
            Self::ContradictoryEvidence(value) => {
                format!("contradictory_evidence = {value}")
            }
        }
    }
}

/// Normalized executor resource bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvestigationBudget {
    /// Maximum resident investigation memory in bytes.
    pub memory_bytes: Option<u64>,
    /// Maximum investigation latency in milliseconds.
    pub latency_millis: Option<u64>,
    /// Maximum number of external retrieval operations.
    pub external_retrievals: Option<u32>,
}

/// Supported investigation response projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReturnProjection {
    /// Calibrated assessment.
    Assessment,
    /// Proof-carrying evidence graph.
    ProofGraph,
    /// Evidence that challenges the assessment.
    CounterEvidence,
    /// Explicit unresolved facts.
    Unknowns,
    /// Ranked next-best evidence requests.
    NextBestEvidence,
}

impl ReturnProjection {
    fn canonical_name(self) -> &'static str {
        match self {
            Self::Assessment => "assessment",
            Self::ProofGraph => "proof_graph",
            Self::CounterEvidence => "counter_evidence",
            Self::Unknowns => "unknowns",
            Self::NextBestEvidence => "next_best_evidence",
        }
    }
}

/// Stable category for declarative investigation parse and semantic failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvestigationErrorCode {
    /// The statement does not match the investigation grammar.
    InvalidSyntax,
    /// The requested investigation operation is unsupported.
    UnsupportedIntent,
    /// The target entity category is unsupported.
    UnsupportedTarget,
    /// A clause that may occur once was repeated.
    DuplicateClause,
    /// A mandatory clause was omitted.
    MissingClause,
    /// A scalar, timestamp, unit, or threshold is invalid.
    InvalidValue,
    /// Multiple declarations assign incompatible values to one contract.
    ConflictingContract,
    /// The statement attempts to embed lower-level Cypher.
    GatewayBoundaryViolation,
}

/// Actionable declarative investigation parsing error.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct InvestigationParseError {
    /// Stable machine-readable error category.
    pub code: InvestigationErrorCode,
    /// Human-readable failure description.
    pub message: String,
    /// Optional guidance for constructing a valid statement.
    pub suggestion: Option<String>,
}

/// Parses and semantically validates a declarative investigation statement.
///
/// The implementation will normalize units and ordering, reject duplicate or
/// conflicting contracts, and prevent embedded Cypher from crossing the
/// agent-safe gateway boundary.
pub fn parse_investigation_query(
    query_text: &str,
) -> Result<InvestigationQuery, InvestigationParseError> {
    let normalized = normalize_whitespace(query_text)?;
    let (intent, target, remainder) = parse_header(&normalized)?;

    if let Some(keyword) = gateway_keyword(remainder) {
        return Err(error(
            InvestigationErrorCode::GatewayBoundaryViolation,
            format!("embedded Cypher clause `{keyword}` is not allowed"),
            "submit lower-level Cypher through the agent-safe gateway instead",
        ));
    }

    let clauses = split_clauses(remainder)?;
    reject_duplicate_clauses(&clauses)?;

    let mut at_time = None;
    let mut requirements = Vec::new();
    let mut allowances = Vec::new();
    let mut budget = None;
    let mut returns = None;

    for (kind, content) in clauses {
        match kind {
            ClauseName::AtTime => at_time = Some(parse_timestamp(content)?),
            ClauseName::Require => requirements = parse_requirements(content)?,
            ClauseName::Allow => allowances = parse_allowances(content)?,
            ClauseName::Budget => budget = Some(parse_budget(content)?),
            ClauseName::Return => returns = Some(parse_returns(content)?),
        }
    }

    let returns = returns.ok_or_else(|| {
        error(
            InvestigationErrorCode::MissingClause,
            "mandatory RETURN clause is missing",
            "add `RETURN assessment` or another supported projection",
        )
    })?;

    Ok(InvestigationQuery {
        intent,
        target,
        at_time,
        requirements,
        allowances,
        budget,
        returns,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClauseName {
    AtTime,
    Require,
    Allow,
    Budget,
    Return,
}

impl ClauseName {
    const ALL: [(Self, &'static str); 5] = [
        (Self::AtTime, "AT TIME"),
        (Self::Require, "REQUIRE"),
        (Self::Allow, "ALLOW"),
        (Self::Budget, "BUDGET"),
        (Self::Return, "RETURN"),
    ];

    fn canonical_name(self) -> &'static str {
        match self {
            Self::AtTime => "AT TIME",
            Self::Require => "REQUIRE",
            Self::Allow => "ALLOW",
            Self::Budget => "BUDGET",
            Self::Return => "RETURN",
        }
    }
}

fn normalize_whitespace(input: &str) -> Result<String, InvestigationParseError> {
    if input.trim().is_empty() {
        return Err(error(
            InvestigationErrorCode::InvalidSyntax,
            "investigation statement cannot be empty",
            "start with `INVESTIGATE <intent> OF <Target>(\"<id>\")`",
        ));
    }
    if !input.is_ascii() {
        return Err(error(
            InvestigationErrorCode::InvalidSyntax,
            "investigation grammar currently accepts ASCII input only",
            "use ASCII identifiers and contract names",
        ));
    }

    let mut normalized = String::new();
    let mut quoted = false;
    let mut pending_space = false;
    for character in input.trim().chars() {
        match character {
            '"' => {
                if pending_space && !normalized.is_empty() && !normalized.ends_with(' ') {
                    normalized.push(' ');
                }
                pending_space = false;
                quoted = !quoted;
                normalized.push(character);
            }
            value if value.is_whitespace() && !quoted => pending_space = true,
            value => {
                if pending_space && !normalized.is_empty() && !normalized.ends_with(' ') {
                    normalized.push(' ');
                }
                pending_space = false;
                normalized.push(value);
            }
        }
    }
    if quoted {
        return Err(error(
            InvestigationErrorCode::InvalidSyntax,
            "unterminated target identifier",
            "close the target identifier with a double quote",
        ));
    }
    Ok(normalized)
}

fn parse_header(
    input: &str,
) -> Result<(InvestigationIntent, InvestigationTarget, &str), InvestigationParseError> {
    const PREFIX: &str = "INVESTIGATE ";
    if !starts_with_ignore_ascii_case(input, PREFIX) {
        return Err(error(
            InvestigationErrorCode::InvalidSyntax,
            "statement must begin with INVESTIGATE",
            "use `INVESTIGATE attribution OF Campaign(\"C-42\")`",
        ));
    }
    let after_prefix = &input[PREFIX.len()..];
    let of_index = find_ignore_ascii_case(after_prefix, " OF ").ok_or_else(|| {
        error(
            InvestigationErrorCode::InvalidSyntax,
            "investigation intent must be followed by OF",
            "use `INVESTIGATE <intent> OF <Target>(\"<id>\")`",
        )
    })?;
    let intent_name = after_prefix[..of_index].trim();
    let intent = match intent_name.to_ascii_lowercase().as_str() {
        "attribution" => InvestigationIntent::Attribution,
        _ => {
            return Err(error(
                InvestigationErrorCode::UnsupportedIntent,
                format!("unsupported investigation intent `{intent_name}`"),
                "use the supported `attribution` intent",
            ));
        }
    };

    let target_text = &after_prefix[of_index + " OF ".len()..];
    let opening_parenthesis = target_text.find('(').ok_or_else(|| {
        error(
            InvestigationErrorCode::InvalidSyntax,
            "target must use `Type(\"identifier\")` syntax",
            "add a quoted stable identifier after the target type",
        )
    })?;
    let target_kind_name = target_text[..opening_parenthesis].trim();
    let kind = match target_kind_name.to_ascii_lowercase().as_str() {
        "campaign" => InvestigationTargetKind::Campaign,
        "actor" => InvestigationTargetKind::Actor,
        "narrative" => InvestigationTargetKind::Narrative,
        "indicator" => InvestigationTargetKind::Indicator,
        _ => {
            return Err(error(
                InvestigationErrorCode::UnsupportedTarget,
                format!("unsupported investigation target `{target_kind_name}`"),
                "use Campaign, Actor, Narrative, or Indicator",
            ));
        }
    };

    let identifier_and_remainder = &target_text[opening_parenthesis + 1..];
    if !identifier_and_remainder.starts_with('"') {
        return Err(error(
            InvestigationErrorCode::InvalidSyntax,
            "target identifier must be double quoted",
            "use `Target(\"stable-id\")`",
        ));
    }
    let closing = identifier_and_remainder[1..]
        .find("\")")
        .map(|index| index + 1)
        .ok_or_else(|| {
            error(
                InvestigationErrorCode::InvalidSyntax,
                "target is missing its closing quote or parenthesis",
                "close the target using `\")`",
            )
        })?;
    let identifier = &identifier_and_remainder[1..closing];
    if identifier.is_empty() || identifier.contains('\\') {
        return Err(error(
            InvestigationErrorCode::InvalidValue,
            "target identifier must be non-empty and cannot contain escapes",
            "provide a stable literal target identifier",
        ));
    }
    let remainder = identifier_and_remainder[closing + 2..].trim();

    Ok((
        intent,
        InvestigationTarget {
            kind,
            identifier: identifier.to_owned(),
        },
        remainder,
    ))
}

fn split_clauses(input: &str) -> Result<Vec<(ClauseName, &str)>, InvestigationParseError> {
    if input.is_empty() {
        return Ok(Vec::new());
    }

    let mut markers = Vec::new();
    for index in 0..input.len() {
        if index > 0 && !input.as_bytes()[index - 1].is_ascii_whitespace() {
            continue;
        }
        for (kind, keyword) in ClauseName::ALL {
            let end = index + keyword.len();
            if end <= input.len()
                && input[index..end].eq_ignore_ascii_case(keyword)
                && (end == input.len() || input.as_bytes()[end].is_ascii_whitespace())
            {
                markers.push((index, end, kind));
                break;
            }
        }
    }

    if markers.is_empty() || markers[0].0 != 0 {
        return Err(error(
            InvestigationErrorCode::InvalidSyntax,
            "unexpected content after investigation target",
            "use only AT TIME, REQUIRE, ALLOW, BUDGET, and RETURN clauses",
        ));
    }

    let mut clauses = Vec::with_capacity(markers.len());
    for (position, (_, content_start, kind)) in markers.iter().enumerate() {
        let content_end = markers
            .get(position + 1)
            .map_or(input.len(), |marker| marker.0);
        let content = input[*content_start..content_end].trim();
        if content.is_empty() {
            return Err(error(
                InvestigationErrorCode::InvalidSyntax,
                format!("{} clause cannot be empty", kind.canonical_name()),
                "provide at least one value for the clause",
            ));
        }
        clauses.push((*kind, content));
    }
    Ok(clauses)
}

fn reject_duplicate_clauses(clauses: &[(ClauseName, &str)]) -> Result<(), InvestigationParseError> {
    for (candidate, _) in ClauseName::ALL {
        if clauses
            .iter()
            .filter(|(kind, _)| *kind == candidate)
            .count()
            > 1
        {
            return Err(error(
                InvestigationErrorCode::DuplicateClause,
                format!("{} clause may appear only once", candidate.canonical_name()),
                "merge all values into one clause",
            ));
        }
    }
    Ok(())
}

fn parse_timestamp(input: &str) -> Result<InvestigationTimestamp, InvestigationParseError> {
    let (date, time) = input
        .split_once('T')
        .map_or((input, None), |(date, time)| (date, Some(time)));
    if !valid_date(date) || time.is_some_and(|value| !valid_utc_time(value)) {
        return Err(invalid_value(
            "AT TIME",
            "expected a valid YYYY-MM-DD date or UTC YYYY-MM-DDTHH:MM:SSZ timestamp",
        ));
    }
    Ok(InvestigationTimestamp(input.to_owned()))
}

fn valid_date(input: &str) -> bool {
    let parts = input.split('-').collect::<Vec<_>>();
    if parts.len() != 3 || parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return false;
    }
    let Ok(year) = parts[0].parse::<u32>() else {
        return false;
    };
    let Ok(month) = parts[1].parse::<u32>() else {
        return false;
    };
    let Ok(day) = parts[2].parse::<u32>() else {
        return false;
    };
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=max_day).contains(&day)
}

fn valid_utc_time(input: &str) -> bool {
    let Some(time) = input.strip_suffix('Z') else {
        return false;
    };
    let parts = time.split(':').collect::<Vec<_>>();
    if parts.len() != 3 || parts.iter().any(|part| part.len() != 2) {
        return false;
    }
    matches!(
        (
            parts[0].parse::<u32>(),
            parts[1].parse::<u32>(),
            parts[2].parse::<u32>(),
        ),
        (Ok(0..=23), Ok(0..=59), Ok(0..=59))
    )
}

fn parse_requirements(input: &str) -> Result<Vec<Requirement>, InvestigationParseError> {
    let mut independent_sources = None;
    let mut source_reliability = None;
    let mut evidence_completeness = None;

    for contract in comma_separated(input, "REQUIRE")? {
        let (name, value) = split_operator(contract, ">=", "requirement")?;
        match name.to_ascii_lowercase().as_str() {
            "independent_sources" => {
                let parsed = parse_positive_u32(value, name)?;
                set_once(&mut independent_sources, parsed, name)?;
            }
            "source_reliability" => {
                let parsed = parse_threshold(value, name)?;
                set_once(&mut source_reliability, parsed, name)?;
            }
            "evidence_completeness" => {
                let parsed = parse_threshold(value, name)?;
                set_once(&mut evidence_completeness, parsed, name)?;
            }
            _ => {
                return Err(invalid_value(name, "unsupported REQUIRE contract name"));
            }
        }
    }

    let mut requirements = Vec::new();
    if let Some(value) = independent_sources {
        requirements.push(Requirement::IndependentSourcesAtLeast(value));
    }
    if let Some(value) = source_reliability {
        requirements.push(Requirement::SourceReliabilityAtLeast(value));
    }
    if let Some(value) = evidence_completeness {
        requirements.push(Requirement::EvidenceCompletenessAtLeast(value));
    }
    Ok(requirements)
}

fn parse_allowances(input: &str) -> Result<Vec<Allowance>, InvestigationParseError> {
    let mut hypotheses = None;
    let mut contradictory_evidence = None;

    for contract in comma_separated(input, "ALLOW")? {
        let (name, value) = split_operator(contract, "=", "allowance")?;
        let parsed = parse_bool(value, name)?;
        match name.to_ascii_lowercase().as_str() {
            "hypotheses" => set_once(&mut hypotheses, parsed, name)?,
            "contradictory_evidence" => {
                set_once(&mut contradictory_evidence, parsed, name)?;
            }
            _ => return Err(invalid_value(name, "unsupported ALLOW contract name")),
        }
    }

    let mut allowances = Vec::new();
    if let Some(value) = hypotheses {
        allowances.push(Allowance::Hypotheses(value));
    }
    if let Some(value) = contradictory_evidence {
        allowances.push(Allowance::ContradictoryEvidence(value));
    }
    Ok(allowances)
}

fn parse_budget(input: &str) -> Result<InvestigationBudget, InvestigationParseError> {
    let mut memory_bytes = None;
    let mut latency_millis = None;
    let mut external_retrievals = None;

    for contract in comma_separated(input, "BUDGET")? {
        let (name, value) = split_operator(contract, "=", "budget")?;
        match name.to_ascii_lowercase().as_str() {
            "memory" => set_once(&mut memory_bytes, parse_memory(value)?, name)?,
            "latency" => set_once(&mut latency_millis, parse_latency(value)?, name)?,
            "external_retrievals" => {
                set_once(&mut external_retrievals, parse_u32(value, name)?, name)?;
            }
            _ => return Err(invalid_value(name, "unsupported BUDGET contract name")),
        }
    }

    Ok(InvestigationBudget {
        memory_bytes,
        latency_millis,
        external_retrievals,
    })
}

fn parse_returns(input: &str) -> Result<Vec<ReturnProjection>, InvestigationParseError> {
    let mut assessment = false;
    let mut proof_graph = false;
    let mut counter_evidence = false;
    let mut unknowns = false;
    let mut next_best_evidence = false;

    for name in comma_separated(input, "RETURN")? {
        let slot = match name.to_ascii_lowercase().as_str() {
            "assessment" => &mut assessment,
            "proof_graph" => &mut proof_graph,
            "counter_evidence" => &mut counter_evidence,
            "unknowns" => &mut unknowns,
            "next_best_evidence" => &mut next_best_evidence,
            _ => return Err(invalid_value(name, "unsupported RETURN projection")),
        };
        if *slot {
            return Err(conflicting_contract(name));
        }
        *slot = true;
    }

    let mut projections = Vec::new();
    if assessment {
        projections.push(ReturnProjection::Assessment);
    }
    if proof_graph {
        projections.push(ReturnProjection::ProofGraph);
    }
    if counter_evidence {
        projections.push(ReturnProjection::CounterEvidence);
    }
    if unknowns {
        projections.push(ReturnProjection::Unknowns);
    }
    if next_best_evidence {
        projections.push(ReturnProjection::NextBestEvidence);
    }
    Ok(projections)
}

fn comma_separated<'a>(
    input: &'a str,
    clause: &str,
) -> Result<Vec<&'a str>, InvestigationParseError> {
    let values = input.split(',').map(str::trim).collect::<Vec<_>>();
    if values.iter().any(|value| value.is_empty()) {
        return Err(error(
            InvestigationErrorCode::InvalidSyntax,
            format!("{clause} contains an empty value"),
            "remove trailing commas and provide every contract value",
        ));
    }
    Ok(values)
}

fn split_operator<'a>(
    input: &'a str,
    operator: &str,
    kind: &str,
) -> Result<(&'a str, &'a str), InvestigationParseError> {
    let (name, value) = input.split_once(operator).ok_or_else(|| {
        error(
            InvestigationErrorCode::InvalidSyntax,
            format!("{kind} `{input}` must use `{operator}`"),
            "use the operator required by the clause grammar",
        )
    })?;
    let name = name.trim();
    let value = value.trim();
    if name.is_empty() || value.is_empty() {
        return Err(error(
            InvestigationErrorCode::InvalidSyntax,
            format!("{kind} name and value cannot be empty"),
            "provide both a contract name and value",
        ));
    }
    Ok((name, value))
}

fn parse_threshold(
    input: &str,
    field: &str,
) -> Result<NormalizedThreshold, InvestigationParseError> {
    let (whole, fraction) = input.split_once('.').map_or((input, ""), |parts| parts);
    if !matches!(whole, "" | "0" | "1")
        || !fraction.chars().all(|value| value.is_ascii_digit())
        || fraction.len() > 6
        || (whole.is_empty() && fraction.is_empty())
    {
        return Err(invalid_value(
            field,
            "threshold must be a decimal between 0 and 1 with at most six decimals",
        ));
    }
    let fraction_value = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<u32>()
            .map_err(|_| invalid_value(field, "threshold is not a valid decimal"))?
            * 10_u32.pow((6 - fraction.len()) as u32)
    };
    let parts = if whole == "1" {
        if fraction_value != 0 {
            return Err(invalid_value(field, "threshold cannot exceed 1"));
        }
        1_000_000
    } else {
        fraction_value
    };
    NormalizedThreshold::from_parts_per_million(parts)
        .map_err(|_| invalid_value(field, "threshold must be between 0 and 1 inclusive"))
}

fn canonical_threshold(value: NormalizedThreshold) -> String {
    if value.parts_per_million() == 1_000_000 {
        return "1".to_owned();
    }
    let mut fraction = format!("{:06}", value.parts_per_million());
    while fraction.ends_with('0') {
        fraction.pop();
    }
    if fraction.is_empty() {
        "0".to_owned()
    } else {
        format!("0.{fraction}")
    }
}

fn parse_memory(input: &str) -> Result<u64, InvestigationParseError> {
    let (amount, unit) = parse_amount_and_unit(input, "memory")?;
    let multiplier = match unit.to_ascii_uppercase().as_str() {
        "B" => 1,
        "KB" => 1024,
        "MB" => 1024 * 1024,
        "GB" => 1024 * 1024 * 1024,
        _ => {
            return Err(invalid_value(
                "memory",
                "supported units are B, KB, MB, and GB",
            ));
        }
    };
    amount
        .checked_mul(multiplier)
        .ok_or_else(|| invalid_value("memory", "memory budget overflows bytes"))
}

fn parse_latency(input: &str) -> Result<u64, InvestigationParseError> {
    let (amount, unit) = parse_amount_and_unit(input, "latency")?;
    let multiplier = match unit.to_ascii_lowercase().as_str() {
        "ms" => 1,
        "s" => 1000,
        _ => return Err(invalid_value("latency", "supported units are ms and s")),
    };
    amount
        .checked_mul(multiplier)
        .ok_or_else(|| invalid_value("latency", "latency budget overflows milliseconds"))
}

fn parse_amount_and_unit<'a>(
    input: &'a str,
    field: &str,
) -> Result<(u64, &'a str), InvestigationParseError> {
    let mut parts = input.split_whitespace();
    let amount = parts
        .next()
        .ok_or_else(|| invalid_value(field, "amount is missing"))?
        .parse::<u64>()
        .map_err(|_| invalid_value(field, "amount must be a non-negative integer"))?;
    let unit = parts
        .next()
        .ok_or_else(|| invalid_value(field, "unit is missing"))?;
    if parts.next().is_some() {
        return Err(invalid_value(field, "expected one amount and one unit"));
    }
    Ok((amount, unit))
}

fn parse_bool(input: &str, field: &str) -> Result<bool, InvestigationParseError> {
    match input.to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(invalid_value(field, "expected true or false")),
    }
}

fn parse_positive_u32(input: &str, field: &str) -> Result<u32, InvestigationParseError> {
    let value = parse_u32(input, field)?;
    if value == 0 {
        return Err(invalid_value(field, "value must be greater than zero"));
    }
    Ok(value)
}

fn parse_u32(input: &str, field: &str) -> Result<u32, InvestigationParseError> {
    input
        .parse::<u32>()
        .map_err(|_| invalid_value(field, "value must be a non-negative integer"))
}

fn set_once<T>(slot: &mut Option<T>, value: T, field: &str) -> Result<(), InvestigationParseError> {
    if slot.is_some() {
        return Err(conflicting_contract(field));
    }
    *slot = Some(value);
    Ok(())
}

fn gateway_keyword(input: &str) -> Option<&str> {
    const KEYWORDS: [&str; 8] = [
        "MATCH", "OPTIONAL", "CREATE", "MERGE", "SET", "DELETE", "REMOVE", "CALL",
    ];
    input
        .split(|value: char| !value.is_ascii_alphanumeric() && value != '_')
        .find_map(|word| {
            KEYWORDS
                .iter()
                .find(|keyword| word.eq_ignore_ascii_case(keyword))
                .copied()
        })
}

fn starts_with_ignore_ascii_case(input: &str, prefix: &str) -> bool {
    input
        .get(..prefix.len())
        .is_some_and(|value| value.eq_ignore_ascii_case(prefix))
}

fn find_ignore_ascii_case(input: &str, needle: &str) -> Option<usize> {
    input
        .as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

fn conflicting_contract(field: &str) -> InvestigationParseError {
    error(
        InvestigationErrorCode::ConflictingContract,
        format!("contract `{field}` is declared more than once"),
        "declare each contract exactly once",
    )
}

fn invalid_value(field: &str, detail: &str) -> InvestigationParseError {
    error(
        InvestigationErrorCode::InvalidValue,
        format!("invalid `{field}` value: {detail}"),
        "use a supported value and normalized unit",
    )
}

fn error(
    code: InvestigationErrorCode,
    message: impl Into<String>,
    suggestion: impl Into<String>,
) -> InvestigationParseError {
    InvestigationParseError {
        code,
        message: message.into(),
        suggestion: Some(suggestion.into()),
    }
}
