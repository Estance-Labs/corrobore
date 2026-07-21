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
//! Question-driven resolution selection contracts (Epic 0022).
//!
//! Selection maps typed investigation intent to tactical, operational, or
//! strategic detail. Ambiguity must be resolved by a stable rule and every
//! decision must carry an audit trace; unsupported or conflicting inputs never
//! receive a silent default.

use serde::{Deserialize, Serialize};

use crate::{GraphError, ResolutionLevel};

/// Typed investigation-question intent used for resolution selection.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum QuestionIntent {
    /// Inspect concrete evidence, accounts, URLs, actions, or timestamps.
    TacticalEvidenceDetail,
    /// Analyze claims, narratives, campaigns, or operational patterns.
    OperationalCampaignAnalysis,
    /// Assess actors, objectives, regions, trends, or strategic impact.
    StrategicObjectiveAssessment,
    /// Preserve an unrecognized classifier output for explicit rejection.
    Unsupported(String),
}

/// Input contract for one question-driven resolution decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolutionSelectionRequest {
    question_ref: String,
    intents: Vec<QuestionIntent>,
    requested_level: Option<ResolutionLevel>,
}

impl ResolutionSelectionRequest {
    /// Creates a validated selection request.
    ///
    /// The final implementation validates the question reference and requires
    /// at least one typed classification so selection cannot fall back silently.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidResolutionSelection`] for invalid request
    /// boundaries.
    pub fn new(
        question_ref: impl Into<String>,
        intents: Vec<QuestionIntent>,
    ) -> Result<Self, GraphError> {
        let question_ref = question_ref.into();
        if question_ref.trim().is_empty() {
            return Err(GraphError::InvalidResolutionSelection(
                "question reference must not be blank".to_owned(),
            ));
        }
        if intents.is_empty() {
            return Err(GraphError::InvalidResolutionSelection(
                "at least one question intent is required".to_owned(),
            ));
        }
        Ok(Self {
            question_ref,
            intents,
            requested_level: None,
        })
    }

    /// Adds an explicit caller-requested resolution for conflict validation.
    #[must_use]
    pub fn with_requested_level(mut self, requested_level: ResolutionLevel) -> Self {
        self.requested_level = Some(requested_level);
        self
    }
}

/// Stable explanation of how the selected level was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolutionSelectionReason {
    /// One inferred level determined the selection.
    Direct,
    /// Multiple inferred levels were resolved to the most detailed level.
    AmbiguousMostDetailed,
    /// An explicit requested level agreed with the inferred selection.
    ExplicitMatch,
}

/// One typed intent-to-resolution mapping captured in the audit trace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentResolutionMapping {
    intent: QuestionIntent,
    level: ResolutionLevel,
}

/// Auditable metadata for one resolution-selection decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolutionSelectionTrace {
    question_ref: String,
    intent_mappings: Vec<IntentResolutionMapping>,
    requested_level: Option<ResolutionLevel>,
    reason: ResolutionSelectionReason,
}

impl ResolutionSelectionTrace {
    /// Returns the stable question reference.
    #[must_use]
    pub fn question_ref(&self) -> &str {
        self.question_ref.as_str()
    }

    /// Returns sorted intent mappings considered by the selector.
    #[must_use]
    pub fn intent_mappings(&self) -> &[IntentResolutionMapping] {
        self.intent_mappings.as_slice()
    }

    /// Returns the caller-requested level, when present.
    #[must_use]
    pub const fn requested_level(&self) -> Option<ResolutionLevel> {
        self.requested_level
    }

    /// Returns the typed selection reason.
    #[must_use]
    pub const fn reason(&self) -> ResolutionSelectionReason {
        self.reason
    }
}

/// Selected resolution and its complete audit trace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolutionSelection {
    selected_level: ResolutionLevel,
    trace: ResolutionSelectionTrace,
}

impl ResolutionSelection {
    /// Returns the selected resolution level.
    #[must_use]
    pub const fn selected_level(&self) -> ResolutionLevel {
        self.selected_level
    }

    /// Returns the audit trace explaining the selection.
    #[must_use]
    pub const fn trace(&self) -> &ResolutionSelectionTrace {
        &self.trace
    }
}

/// Selects graph resolution from typed question intent.
///
/// The implementation will map all supported intents, sort and deduplicate
/// mappings, choose the most detailed level for ambiguity, validate any
/// explicit request, and return a deterministic audit trace.
///
/// # Errors
///
/// Returns [`GraphError::InvalidResolutionSelection`] for unsupported intents
/// or conflicts.
pub fn select_question_resolution(
    request: &ResolutionSelectionRequest,
) -> Result<ResolutionSelection, GraphError> {
    let mut intent_mappings = request
        .intents
        .iter()
        .map(|intent| {
            let level = match intent {
                QuestionIntent::TacticalEvidenceDetail => ResolutionLevel::Tactical,
                QuestionIntent::OperationalCampaignAnalysis => ResolutionLevel::Operational,
                QuestionIntent::StrategicObjectiveAssessment => ResolutionLevel::Strategic,
                QuestionIntent::Unsupported(classification) => {
                    return Err(GraphError::InvalidResolutionSelection(format!(
                        "unsupported question intent classification: {classification}"
                    )));
                }
            };
            Ok(IntentResolutionMapping {
                intent: intent.clone(),
                level,
            })
        })
        .collect::<Result<Vec<_>, GraphError>>()?;

    intent_mappings.sort_by(|left, right| left.intent.cmp(&right.intent));
    intent_mappings.dedup();

    let mut inferred_levels = intent_mappings
        .iter()
        .map(|mapping| mapping.level)
        .collect::<Vec<_>>();
    inferred_levels.sort();
    inferred_levels.dedup();

    let selected_level = inferred_levels.first().copied().ok_or_else(|| {
        GraphError::InvalidResolutionSelection(
            "at least one supported question intent is required".to_owned(),
        )
    })?;

    if let Some(requested_level) = request.requested_level
        && requested_level != selected_level
    {
        return Err(GraphError::InvalidResolutionSelection(format!(
            "requested resolution {requested_level:?} conflicts with inferred resolution \
             {selected_level:?}"
        )));
    }

    let reason = if request.requested_level.is_some() {
        ResolutionSelectionReason::ExplicitMatch
    } else if inferred_levels.len() > 1 {
        ResolutionSelectionReason::AmbiguousMostDetailed
    } else {
        ResolutionSelectionReason::Direct
    };

    Ok(ResolutionSelection {
        selected_level,
        trace: ResolutionSelectionTrace {
            question_ref: request.question_ref.clone(),
            intent_mappings,
            requested_level: request.requested_level,
            reason,
        },
    })
}
