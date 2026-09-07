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
//! The deterministic bilingual template compiler.
//!
//! Module boundary: this module maps recurring French and English request
//! shapes to envelopes. It is the baseline a model is measured against and the
//! fallback when no model is configured. It never guesses: an unfamiliar
//! request is `unsupported`, an ambiguous one asks, and a request it cannot
//! honour under the caller's boundary abstains with a reason.
//!
//! Literals the user wrote (names, identifiers, quoted phrases) are carried
//! verbatim; only the request shape is interpreted.

use serde_json::json;

use crate::{Action, Envelope, Language, NlqCompiler, NlqRequest};

/// Default row limit for list reads.
const DEFAULT_LIMIT: u32 = 50;

/// The bilingual template compiler.
#[derive(Clone, Copy, Debug, Default)]
pub struct TemplateCompiler;

impl NlqCompiler for TemplateCompiler {
    fn name(&self) -> &'static str {
        "template"
    }

    fn compile(&self, request: &NlqRequest) -> Envelope {
        let language = request
            .language()
            .cloned()
            .unwrap_or_else(|| detect_language(request.text()));
        let text = normalize(request.text());
        if text.is_empty() {
            return Envelope::new(
                language,
                Action::Unsupported {
                    reason: "the request is empty".to_owned(),
                },
                "empty_request",
            );
        }
        let lowered = fold(&text);
        let words: Vec<&str> = lowered.split_whitespace().collect();

        if let Some(envelope) = destructive(&words, &language) {
            return envelope;
        }
        if let Some(envelope) = investigation(&text, &lowered, &language) {
            return envelope;
        }
        if let Some(envelope) = remember(request, &text, &lowered, &language) {
            return envelope;
        }
        if let Some(envelope) = recall(&text, &lowered, &language) {
            return envelope;
        }
        if let Some(envelope) = relationship_question(&text, &lowered, &language) {
            return envelope;
        }
        if let Some(envelope) = count(&lowered, &language) {
            return envelope;
        }
        if let Some(envelope) = list(&lowered, &language) {
            return envelope;
        }
        if let Some(envelope) = write_shaped(request, &lowered, &language) {
            return envelope;
        }
        if let Some(envelope) = tell_me_about(&text, &lowered, &language) {
            return envelope;
        }
        Envelope::new(
            language,
            Action::Unsupported {
                reason: "the request does not match a shape this compiler understands".to_owned(),
            },
            "no_template",
        )
    }
}

/// Trim, drop trailing punctuation, collapse whitespace.
fn normalize(text: &str) -> String {
    let trimmed = text.trim().trim_end_matches(['?', '.', '!', ' ', '\u{a0}']);
    trimmed.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Lower-case and strip the accents French uses, for matching only.
fn fold(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'à' | 'â' | 'ä' => 'a',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            '’' => '\'',
            other => other,
        })
        .collect::<String>()
        .to_lowercase()
}

/// Score French against English cue words.
fn detect_language(text: &str) -> Language {
    let lowered = fold(text);
    let words: Vec<&str> = lowered
        .split(|character: char| {
            !character.is_alphanumeric() && character != '\'' && character != '-'
        })
        .filter(|word| !word.is_empty())
        .collect();
    const FRENCH: [&str; 22] = [
        "quels",
        "quelles",
        "quel",
        "quelle",
        "liste",
        "montre",
        "affiche",
        "combien",
        "enquete",
        "sais-tu",
        "retiens",
        "souviens-toi",
        "supprime",
        "efface",
        "parle-moi",
        "les",
        "des",
        "du",
        "de",
        "la",
        "le",
        "que",
    ];
    const ENGLISH: [&str; 20] = [
        "which",
        "what",
        "list",
        "show",
        "display",
        "how",
        "many",
        "investigate",
        "know",
        "remember",
        "delete",
        "drop",
        "tell",
        "the",
        "of",
        "about",
        "first",
        "do",
        "you",
        "me",
    ];
    let mut french = 0;
    let mut english = 0;
    for word in &words {
        let word = word.trim_start_matches("l'").trim_start_matches("d'");
        if FRENCH.contains(&word) {
            french += 1;
        }
        if ENGLISH.contains(&word) {
            english += 1;
        }
    }
    if french >= english {
        Language::Fr
    } else {
        Language::En
    }
}

/// Node labels by surface form, in both languages. Longest forms first so
/// "threat actors" wins over "actors".
const LABELS: [(&str, &str); 34] = [
    ("acteurs de menace", "ThreatActor"),
    ("acteur de menace", "ThreatActor"),
    ("threat actors", "ThreatActor"),
    ("threat actor", "ThreatActor"),
    ("logiciels malveillants", "Malware"),
    ("logiciel malveillant", "Malware"),
    ("malwares", "Malware"),
    ("malware", "Malware"),
    ("indicateurs", "Indicator"),
    ("indicateur", "Indicator"),
    ("indicators", "Indicator"),
    ("indicator", "Indicator"),
    ("campagnes", "Campaign"),
    ("campagne", "Campaign"),
    ("campaigns", "Campaign"),
    ("campaign", "Campaign"),
    ("identites", "Identity"),
    ("identite", "Identity"),
    ("identities", "Identity"),
    ("identity", "Identity"),
    ("infrastructures", "Infrastructure"),
    ("infrastructure", "Infrastructure"),
    ("vulnerabilites", "Vulnerability"),
    ("vulnerabilite", "Vulnerability"),
    ("vulnerabilities", "Vulnerability"),
    ("vulnerability", "Vulnerability"),
    ("outils", "Tool"),
    ("outil", "Tool"),
    ("tools", "Tool"),
    ("tool", "Tool"),
    ("rapports", "Report"),
    ("rapport", "Report"),
    ("reports", "Report"),
    ("report", "Report"),
];

/// Relationship verbs by surface form.
const VERBS: [(&str, &str); 8] = [
    ("utilisent", "USES"),
    ("utilise", "USES"),
    ("uses", "USES"),
    ("use", "USES"),
    ("ciblent", "TARGETS"),
    ("cible", "TARGETS"),
    ("targets", "TARGETS"),
    ("target", "TARGETS"),
];

const ARTICLES: [&str; 9] = ["les", "des", "le", "la", "l'", "the", "a", "an", "of"];

/// Find the label whose surface form starts `text`; returns the label and the
/// remainder after the form.
fn label_prefix(text: &str) -> Option<(&'static str, &str)> {
    for (form, label) in LABELS {
        if let Some(rest) = text.strip_prefix(form)
            && (rest.is_empty() || rest.starts_with(' '))
        {
            return Some((label, rest.trim_start()));
        }
    }
    None
}

/// The label whose surface form is exactly `text` (articles stripped).
fn label_exact(text: &str) -> Option<&'static str> {
    let stripped = strip_articles(text);
    label_prefix(stripped).and_then(|(label, rest)| rest.is_empty().then_some(label))
}

fn strip_articles(text: &str) -> &str {
    let mut rest = text.trim();
    loop {
        let mut changed = false;
        for article in ARTICLES {
            if let Some(after) = rest.strip_prefix(article)
                && (article.ends_with('\'') || after.starts_with(' '))
            {
                rest = after.trim_start();
                changed = true;
                break;
            }
        }
        if !changed {
            return rest;
        }
    }
}

/// Escape a literal for a single-quoted Cypher string.
fn cypher_literal(value: &str) -> String {
    cypher_parser::escape_string_literal(value)
}

/// Take a quoted literal (`"..."`) from the original text, or the last word.
fn literal_from(original: &str, lowered_tail: &str) -> Option<String> {
    if let Some(start) = original.find('"') {
        let rest = &original[start + 1..];
        let end = rest.find('"')?;
        return Some(rest[..end].to_owned());
    }
    let tail = lowered_tail.trim();
    if tail.is_empty() {
        return None;
    }
    // Recover the original casing of the trailing token from the source text.
    let last = tail.split_whitespace().last()?;
    original
        .split_whitespace()
        .rev()
        .map(|word| word.trim_matches(['"', '?', '.', '!', ',']))
        .find(|word| fold(word) == last)
        .map(str::to_owned)
}

fn destructive(words: &[&str], language: &Language) -> Option<Envelope> {
    const DESTRUCTIVE: [&str; 8] = [
        "supprime", "efface", "detruis", "delete", "drop", "truncate", "purge", "wipe",
    ];
    let verb = words.first()?;
    DESTRUCTIVE.contains(verb).then(|| {
        Envelope::new(
            language.clone(),
            Action::Unsupported {
                reason: "destructive requests are not compiled from natural language; use an authorized, reviewed mutation".to_owned(),
            },
            "destructive_request",
        )
    })
}

fn investigation(original: &str, lowered: &str, language: &Language) -> Option<Envelope> {
    const LEADS: [&str; 4] = ["enquete sur", "enquete", "investigate", "investigation on"];
    let mut rest = None;
    for lead in LEADS {
        if let Some(after) = lowered.strip_prefix(lead) {
            rest = Some(after.trim_start());
            break;
        }
    }
    let rest = strip_articles(rest?);
    let (intent, after_intent) = rest.split_once(' ')?;
    let after_intent = strip_articles(
        after_intent
            .trim_start_matches("de la")
            .trim_start_matches("de l'")
            .trim_start(),
    );
    let (target_kind, after_target) = label_prefix(strip_articles(after_intent))?;
    let target_kind = match target_kind {
        "Campaign" => "Campaign",
        "ThreatActor" => "Actor",
        "Indicator" => "Indicator",
        _ => {
            return Some(Envelope::new(
                language.clone(),
                Action::Unsupported {
                    reason: format!("investigations do not target {target_kind}"),
                },
                "unsupported_target",
            ));
        }
    };
    let identifier = literal_from(original, after_target)?;
    if intent != "attribution" {
        return Some(Envelope::new(
            language.clone(),
            Action::Unsupported {
                reason: format!("the investigation grammar supports `attribution`, not `{intent}`"),
            },
            "unsupported_intent",
        ));
    }
    let identifier = identifier.replace('"', "");
    Some(Envelope::new(
        language.clone(),
        Action::Investigation {
            statement: format!(
                "INVESTIGATE attribution OF {target_kind}(\"{identifier}\") RETURN assessment, counter_evidence, unknowns, next_best_evidence"
            ),
        },
        "compiled",
    ))
}

fn remember(
    request: &NlqRequest,
    original: &str,
    lowered: &str,
    language: &Language,
) -> Option<Envelope> {
    const LEADS: [&str; 5] = [
        "retiens que",
        "souviens-toi que",
        "note que",
        "remember that",
        "note that",
    ];
    let lead = LEADS.iter().find(|lead| lowered.starts_with(*lead))?;
    if !request.writes_allowed() {
        return Some(Envelope::new(
            language.clone(),
            Action::Abstain {
                reason: "remembering writes to memory and the task did not allow writes".to_owned(),
            },
            "write_not_permitted",
        ));
    }
    let statement = original[lead.len()..].trim();
    // "(source <ref>)" names the evidence; the caller must have supplied it.
    let (content, cited) = match statement.rfind("(source ") {
        Some(index) => {
            let reference = statement[index + "(source ".len()..]
                .trim_end_matches(')')
                .trim();
            (statement[..index].trim(), Some(reference.to_owned()))
        }
        None => (statement, None),
    };
    let content = content.trim_matches('"').to_owned();
    // A cited source is only usable when the caller actually supplied it; a
    // reference the text invented is refused rather than laundered into
    // provenance.
    if let Some(reference) = &cited
        && !request
            .evidence_refs()
            .iter()
            .any(|known| known == reference)
    {
        return Some(Envelope::new(
            language.clone(),
            Action::Abstain {
                reason: format!(
                    "the cited source `{reference}` was not supplied by the caller; only caller-supplied evidence can back a memory"
                ),
            },
            "unknown_evidence",
        ));
    }
    let source = cited.or_else(|| request.evidence_refs().first().cloned());
    let Some(source) = source else {
        return Some(Envelope::new(
            language.clone(),
            Action::ClarificationRequired {
                question: match language {
                    Language::Fr => {
                        "Quelle source appuie cette observation ? Indiquez une référence de preuve."
                            .to_owned()
                    }
                    _ => "Which source supports this observation? Supply an evidence reference."
                        .to_owned(),
                },
                options: vec![],
            },
            "no_evidence",
        ));
    };
    Some(
        Envelope::new(
            language.clone(),
            Action::MemoryOperation {
                operation: "remember".to_owned(),
                input: json!({
                    "identity_key": null,
                    "kind": "observation",
                    "schema_version": "v1",
                    "content": {"format": "text", "value": content},
                    "provenance": [{"source_id": source, "locator": null, "observed_at": null}],
                    "confidence": null,
                    "valid_from": null,
                    "valid_until": null,
                    "expires_at": null,
                    "tags": []
                }),
            },
            "compiled",
        )
        .with_evidence_refs([source]),
    )
}

fn recall(original: &str, lowered: &str, language: &Language) -> Option<Envelope> {
    const LEADS: [&str; 6] = [
        "que sais-tu de",
        "que sais-tu sur",
        "qu'est-ce que tu sais de",
        "qu'est-ce que tu sais sur",
        "what do you know about",
        "what is known about",
    ];
    let lead = LEADS.iter().find(|lead| lowered.starts_with(*lead))?;
    let topic = original[lead.len()..].trim();
    let objective = objective_tokens(topic);
    if objective.is_empty() {
        return None;
    }
    Some(
        Envelope::new(
            language.clone(),
            Action::MemoryOperation {
                operation: "recall".to_owned(),
                input: json!({
                    "objective": objective,
                    "seed_ids": [],
                    "limits": {
                        "max_items": 20,
                        "max_depth": 2,
                        "max_payload_bytes": 65536,
                        "max_cost": 500,
                        "timeout_ms": 2000,
                        "supernode_threshold": 1000
                    },
                    "page_token": null
                }),
            },
            "compiled",
        )
        .with_limit(20),
    )
}

/// A language-neutral objective: content words only, in order, so the French
/// and English surfaces of one request recall the same thing.
fn objective_tokens(topic: &str) -> String {
    const STOP: [&str; 14] = [
        "l'", "d'", "de", "du", "des", "la", "le", "les", "the", "of", "a", "an", "about", "sur",
    ];
    topic
        .split_whitespace()
        .flat_map(|word| {
            let word = word.trim_matches(['"', '?', '.', '!', ',']);
            // Split elided articles: "l'infrastructure" -> "infrastructure".
            match word.split_once('\'') {
                Some((prefix, rest)) if prefix.len() <= 2 => vec![rest],
                _ => vec![word],
            }
        })
        .filter(|word| !word.is_empty() && !STOP.contains(&fold(word).as_str()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn relationship_question(original: &str, lowered: &str, language: &Language) -> Option<Envelope> {
    const LEADS: [&str; 5] = ["quels", "quelles", "which", "what", "quel"];
    let lead = LEADS.iter().find(|lead| lowered.starts_with(*lead))?;
    let rest = lowered[lead.len()..].trim_start();
    let (subject_label, after_subject) = label_prefix(rest)?;
    let (verb_form, relationship) = VERBS.iter().find(|(form, _)| {
        after_subject.starts_with(form) && after_subject[form.len()..].starts_with(' ')
    })?;
    let after_verb = strip_articles(after_subject[verb_form.len()..].trim_start());
    let (object_label, after_object) = label_prefix(after_verb)?;
    let literal = literal_from(original, after_object)?;
    Some(
        Envelope::new(
            language.clone(),
            Action::CypherRead {
                query: format!(
                    "MATCH (a:{subject_label})-[r:{relationship}]->(m:{object_label}) WHERE m.name = {} RETURN a.name LIMIT {DEFAULT_LIMIT}",
                    cypher_literal(&literal)
                ),
            },
            "compiled",
        )
        .with_limit(DEFAULT_LIMIT),
    )
}

fn count(lowered: &str, language: &Language) -> Option<Envelope> {
    const LEADS: [&str; 3] = ["combien de", "combien d'", "how many"];
    let lead = LEADS.iter().find(|lead| lowered.starts_with(*lead))?;
    let label = label_exact(lowered[lead.len()..].trim())?;
    Some(Envelope::new(
        language.clone(),
        Action::CypherRead {
            query: format!("MATCH (n:{label}) RETURN count(n)"),
        },
        "compiled",
    ))
}

fn list(lowered: &str, language: &Language) -> Option<Envelope> {
    const LEADS: [&str; 6] = ["liste", "montre", "affiche", "list", "show", "display"];
    let lead = LEADS
        .iter()
        .find(|lead| lowered.starts_with(*lead) && lowered[lead.len()..].starts_with(' '))?;
    let mut rest = strip_articles(lowered[lead.len()..].trim_start());
    let mut limit = DEFAULT_LIMIT;
    // "les 20 premiers X" / "the first 20 X"
    for filler in ["first", "premiers", "premieres", "top"] {
        rest = strip_articles(rest.strip_prefix(filler).map_or(rest, str::trim_start));
    }
    if let Some((number, after)) = rest.split_once(' ')
        && let Ok(parsed) = number.parse::<u32>()
        && parsed > 0
    {
        limit = parsed;
        rest = after.trim_start();
        for filler in ["first", "premiers", "premieres"] {
            rest = strip_articles(rest.strip_prefix(filler).map_or(rest, str::trim_start));
        }
    }
    let label = label_exact(rest)?;
    Some(
        Envelope::new(
            language.clone(),
            Action::CypherRead {
                query: format!("MATCH (n:{label}) RETURN n LIMIT {limit}"),
            },
            "compiled",
        )
        .with_limit(limit),
    )
}

fn write_shaped(request: &NlqRequest, lowered: &str, language: &Language) -> Option<Envelope> {
    const LEADS: [&str; 8] = [
        "marque",
        "mets a jour",
        "met a jour",
        "modifie",
        "mark",
        "update",
        "set",
        "change",
    ];
    LEADS.iter().find(|lead| lowered.starts_with(*lead))?;
    Some(if request.writes_allowed() {
        Envelope::new(
            language.clone(),
            Action::Unsupported {
                reason: "free-form updates are not compiled from natural language; propose the mutation as Cypher for review".to_owned(),
            },
            "no_template",
        )
    } else {
        Envelope::new(
            language.clone(),
            Action::Abstain {
                reason: "the request would change the graph and the task did not allow writes"
                    .to_owned(),
            },
            "write_not_permitted",
        )
    })
}

fn tell_me_about(original: &str, lowered: &str, language: &Language) -> Option<Envelope> {
    const LEADS: [&str; 4] = ["parle-moi de", "parle moi de", "tell me about", "about"];
    let lead = LEADS.iter().find(|lead| lowered.starts_with(*lead))?;
    let topic = original[lead.len()..].trim();
    Some(Envelope::new(
        language.clone(),
        Action::ClarificationRequired {
            question: match language {
                Language::Fr => format!(
                    "Que voulez-vous sur « {topic} » : les enregistrements du graphe, la mémoire de travail, ou une enquête d'attribution ?"
                ),
                _ => format!(
                    "What do you want about \"{topic}\": graph records, working memory, or an attribution investigation?"
                ),
            },
            options: vec![
                "cypher_read".to_owned(),
                "memory_operation".to_owned(),
                "investigation".to_owned(),
            ],
        },
        "ambiguous_request",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding_strips_french_accents_for_matching_only() {
        assert_eq!(fold("Enquête sur l'Identité"), "enquete sur l'identite");
        assert_eq!(
            normalize("  Liste   les indicateurs ? "),
            "Liste les indicateurs"
        );
    }

    #[test]
    fn literals_keep_their_case_and_quotes_win() {
        assert_eq!(
            literal_from("Quels acteurs utilisent le malware X-Agent", "x-agent"),
            Some("X-Agent".to_owned())
        );
        assert_eq!(
            literal_from(
                "Which campaigns target the identity \"Ministry of Energy\"",
                "\"ministry of energy\""
            ),
            Some("Ministry of Energy".to_owned())
        );
    }

    #[test]
    fn objectives_drop_articles_in_both_languages() {
        assert_eq!(
            objective_tokens("l'infrastructure d'APT28"),
            "infrastructure APT28"
        );
        assert_eq!(
            objective_tokens("the infrastructure of APT28"),
            "infrastructure APT28"
        );
    }
}
