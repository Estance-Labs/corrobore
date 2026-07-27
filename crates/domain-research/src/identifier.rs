// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Scholarly identifier normalization.
//!
//! Normalization is deterministic, offline, and format-level: it canonicalizes
//! shape and verifies self-contained check digits where the scheme defines one.
//! It never resolves an identifier over the network, so it can prove that a
//! string is well-formed but not that the work it names exists.

use serde::{Deserialize, Serialize};

/// Scholarly identifier system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentifierSystem {
    /// Digital Object Identifier.
    Doi,
    /// arXiv preprint identifier.
    ArXiv,
    /// ORCID researcher identifier.
    Orcid,
    /// Research Organization Registry identifier.
    Ror,
    /// International Standard Serial Number.
    Issn,
    /// PubMed identifier.
    PubMed,
}

impl IdentifierSystem {
    /// Resolves a system identifier string.
    #[must_use]
    pub fn from_identifier(identifier: &str) -> Option<Self> {
        let system = match identifier {
            "doi" => Self::Doi,
            "arxiv" => Self::ArXiv,
            "orcid" => Self::Orcid,
            "ror" => Self::Ror,
            "issn" => Self::Issn,
            "pubmed" => Self::PubMed,
            _ => return None,
        };
        Some(system)
    }

    /// Returns the stable system string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Doi => "doi",
            Self::ArXiv => "arxiv",
            Self::Orcid => "orcid",
            Self::Ror => "ror",
            Self::Issn => "issn",
            Self::PubMed => "pubmed",
        }
    }
}

/// Strips a case-insensitive prefix when present.
fn strip_prefix_ci<'a>(value: &'a str, prefix: &str) -> &'a str {
    if value.len() >= prefix.len() && value[..prefix.len()].eq_ignore_ascii_case(prefix) {
        &value[prefix.len()..]
    } else {
        value
    }
}

/// Removes every scheme, host, and scheme-like prefix a scheme is commonly
/// pasted with.
fn strip_known_prefixes<'a>(value: &'a str, prefixes: &[&str]) -> &'a str {
    let mut current = value;
    // Prefixes stack, for example `https://doi.org/` then `doi:`.
    let mut changed = true;
    while changed {
        changed = false;
        for prefix in prefixes {
            let stripped = strip_prefix_ci(current, prefix);
            if stripped.len() != current.len() {
                current = stripped;
                changed = true;
            }
        }
    }
    current
}

fn normalize_doi(value: &str) -> Option<String> {
    let candidate = strip_known_prefixes(
        value,
        &[
            "https://doi.org/",
            "http://doi.org/",
            "https://dx.doi.org/",
            "http://dx.doi.org/",
            "doi:",
        ],
    )
    .trim();

    // A DOI is `10.<registrant>/<suffix>` with a non-empty suffix.
    let rest = candidate.strip_prefix("10.")?;
    let (registrant, suffix) = rest.split_once('/')?;
    if registrant.is_empty() || !registrant.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if suffix.is_empty() || suffix.contains(char::is_whitespace) {
        return None;
    }

    // DOIs are case-insensitive; lowercase is the canonical presentation.
    Some(format!("10.{registrant}/{}", suffix.to_ascii_lowercase()))
}

fn normalize_arxiv(value: &str) -> Option<String> {
    let candidate = strip_known_prefixes(
        value,
        &["https://arxiv.org/abs/", "http://arxiv.org/abs/", "arxiv:"],
    )
    .trim();

    if candidate.is_empty() || candidate.contains(char::is_whitespace) {
        return None;
    }

    // Modern form: YYMM.NNNN or YYMM.NNNNN, optionally versioned.
    if let Some((head, tail)) = candidate.split_once('.') {
        let (number, version) = split_version(tail);
        let modern = head.len() == 4
            && head.bytes().all(|byte| byte.is_ascii_digit())
            && (4..=5).contains(&number.len())
            && number.bytes().all(|byte| byte.is_ascii_digit());
        if modern {
            return Some(match version {
                Some(version) => format!("{head}.{number}v{version}"),
                None => format!("{head}.{number}"),
            });
        }
    }

    // Legacy form: archive[.subclass]/YYMMNNN, optionally versioned.
    let (archive, tail) = candidate.split_once('/')?;
    let (number, version) = split_version(tail);
    let legacy = !archive.is_empty()
        && archive
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic() || byte == b'.' || byte == b'-')
        && number.len() == 7
        && number.bytes().all(|byte| byte.is_ascii_digit());
    if !legacy {
        return None;
    }

    let archive = archive.to_ascii_lowercase();
    Some(match version {
        Some(version) => format!("{archive}/{number}v{version}"),
        None => format!("{archive}/{number}"),
    })
}

/// Splits a trailing `vN` version marker.
fn split_version(value: &str) -> (&str, Option<&str>) {
    if let Some((number, version)) = value.rsplit_once('v')
        && !version.is_empty()
        && version.bytes().all(|byte| byte.is_ascii_digit())
    {
        return (number, Some(version));
    }
    (value, None)
}

/// ISO 7064 MOD 11-2 check character used by ORCID.
fn orcid_check_character(digits: &[u8]) -> char {
    let mut total: u32 = 0;
    for digit in digits {
        total = (total + u32::from(*digit)) * 2;
    }
    let remainder = total % 11;
    let result = (12 - remainder) % 11;
    if result == 10 {
        'X'
    } else {
        char::from(b'0' + u8::try_from(result).unwrap_or(0))
    }
}

fn normalize_orcid(value: &str) -> Option<String> {
    let candidate = strip_known_prefixes(
        value,
        &["https://orcid.org/", "http://orcid.org/", "orcid:"],
    )
    .trim();

    let compact: String = candidate.chars().filter(|c| *c != '-').collect();
    if compact.len() != 16 {
        return None;
    }

    let bytes = compact.as_bytes();
    if !bytes[..15].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let provided = bytes[15].to_ascii_uppercase();
    if !provided.is_ascii_digit() && provided != b'X' {
        return None;
    }

    let digits: Vec<u8> = bytes[..15].iter().map(|byte| byte - b'0').collect();
    if orcid_check_character(&digits) != char::from(provided) {
        return None;
    }

    let compact = compact.to_ascii_uppercase();
    Some(format!(
        "{}-{}-{}-{}",
        &compact[0..4],
        &compact[4..8],
        &compact[8..12],
        &compact[12..16]
    ))
}

fn normalize_ror(value: &str) -> Option<String> {
    let candidate =
        strip_known_prefixes(value, &["https://ror.org/", "http://ror.org/", "ror:"]).trim();

    // A ROR id is `0` followed by eight Crockford base32 characters, which
    // exclude i, l, o, and u.
    if candidate.len() != 9 {
        return None;
    }
    let lowered = candidate.to_ascii_lowercase();
    let mut chars = lowered.chars();
    if chars.next() != Some('0') {
        return None;
    }
    if !chars.all(|c| c.is_ascii_digit() || (c.is_ascii_lowercase() && !"ilou".contains(c))) {
        return None;
    }
    Some(lowered)
}

fn normalize_issn(value: &str) -> Option<String> {
    let candidate = strip_known_prefixes(value, &["issn:", "issn "]).trim();
    let compact: String = candidate.chars().filter(|c| *c != '-').collect();
    if compact.len() != 8 {
        return None;
    }

    let bytes = compact.as_bytes();
    if !bytes[..7].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let provided = bytes[7].to_ascii_uppercase();
    if !provided.is_ascii_digit() && provided != b'X' {
        return None;
    }

    // Mod-11 check digit with descending weights 8 through 2.
    let mut sum: u32 = 0;
    for (index, byte) in bytes[..7].iter().enumerate() {
        let weight = 8 - u32::try_from(index).unwrap_or(0);
        sum += u32::from(byte - b'0') * weight;
    }
    let remainder = sum % 11;
    let expected = if remainder == 0 { 0 } else { 11 - remainder };
    let expected = if expected == 10 {
        'X'
    } else {
        char::from(b'0' + u8::try_from(expected).unwrap_or(0))
    };
    if expected != char::from(provided) {
        return None;
    }

    let compact = compact.to_ascii_uppercase();
    Some(format!("{}-{}", &compact[0..4], &compact[4..8]))
}

fn normalize_pubmed(value: &str) -> Option<String> {
    let candidate = strip_known_prefixes(value, &["pmid:", "pmid ", "pubmed:"]).trim();
    if candidate.is_empty() || candidate.len() > 8 {
        return None;
    }
    if !candidate.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    // Strip insignificant leading zeros so one identifier has one form.
    let trimmed = candidate.trim_start_matches('0');
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

/// Built-in `research.identifier_normalize`: canonicalizes a scholarly
/// identifier, or returns `None` when it is not well-formed for the system.
///
/// Deterministic and offline: the same input always yields the same output and
/// no network resolution is attempted.
#[must_use]
pub fn research_identifier_normalize(system: IdentifierSystem, value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    match system {
        IdentifierSystem::Doi => normalize_doi(value),
        IdentifierSystem::ArXiv => normalize_arxiv(value),
        IdentifierSystem::Orcid => normalize_orcid(value),
        IdentifierSystem::Ror => normalize_ror(value),
        IdentifierSystem::Issn => normalize_issn(value),
        IdentifierSystem::PubMed => normalize_pubmed(value),
    }
}

/// Built-in `research.identifier_is_valid`: reports whether an identifier is
/// well-formed for its system.
#[must_use]
pub fn research_identifier_is_valid(system: IdentifierSystem, value: &str) -> bool {
    research_identifier_normalize(system, value).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doi_normalization_strips_prefixes_and_lowercases_suffix() {
        let expected = Some("10.1000/abc.123".to_owned());
        assert_eq!(
            research_identifier_normalize(IdentifierSystem::Doi, "https://doi.org/10.1000/ABC.123"),
            expected
        );
        assert_eq!(
            research_identifier_normalize(IdentifierSystem::Doi, "doi:10.1000/abc.123"),
            expected
        );
        assert_eq!(
            research_identifier_normalize(IdentifierSystem::Doi, "10.1000/Abc.123"),
            expected
        );
    }

    #[test]
    fn doi_normalization_rejects_malformed_values() {
        for invalid in [
            "11.1000/abc",
            "10./abc",
            "10.1000/",
            "10.abc/def",
            "nonsense",
        ] {
            assert!(
                research_identifier_normalize(IdentifierSystem::Doi, invalid).is_none(),
                "expected {invalid} to be rejected"
            );
        }
    }

    #[test]
    fn arxiv_normalization_handles_modern_legacy_and_versions() {
        assert_eq!(
            research_identifier_normalize(IdentifierSystem::ArXiv, "arXiv:2601.01234"),
            Some("2601.01234".to_owned())
        );
        assert_eq!(
            research_identifier_normalize(
                IdentifierSystem::ArXiv,
                "https://arxiv.org/abs/2601.01234v2"
            ),
            Some("2601.01234v2".to_owned())
        );
        assert_eq!(
            research_identifier_normalize(IdentifierSystem::ArXiv, "math.GT/0309136"),
            Some("math.gt/0309136".to_owned())
        );
        assert!(research_identifier_normalize(IdentifierSystem::ArXiv, "2601.1").is_none());
    }

    #[test]
    fn orcid_normalization_verifies_the_iso_7064_check_digit() {
        // Valid ORCID with an X check character.
        assert_eq!(
            research_identifier_normalize(IdentifierSystem::Orcid, "0000-0002-1825-0097"),
            Some("0000-0002-1825-0097".to_owned())
        );
        assert_eq!(
            research_identifier_normalize(
                IdentifierSystem::Orcid,
                "https://orcid.org/0000000218250097"
            ),
            Some("0000-0002-1825-0097".to_owned())
        );
        // A single transposed digit fails the checksum.
        assert!(
            research_identifier_normalize(IdentifierSystem::Orcid, "0000-0002-1825-0098").is_none()
        );
    }

    #[test]
    fn issn_normalization_verifies_the_mod_11_check_digit() {
        assert_eq!(
            research_identifier_normalize(IdentifierSystem::Issn, "0378-5955"),
            Some("0378-5955".to_owned())
        );
        assert_eq!(
            research_identifier_normalize(IdentifierSystem::Issn, "03785955"),
            Some("0378-5955".to_owned())
        );
        assert!(research_identifier_normalize(IdentifierSystem::Issn, "0378-5954").is_none());
    }

    #[test]
    fn ror_normalization_rejects_ambiguous_characters() {
        assert_eq!(
            research_identifier_normalize(IdentifierSystem::Ror, "https://ror.org/02mhbdp94"),
            Some("02mhbdp94".to_owned())
        );
        // `i`, `l`, `o`, and `u` are excluded from the ROR alphabet.
        assert!(research_identifier_normalize(IdentifierSystem::Ror, "02mhbdpi4").is_none());
        // A ROR id always starts with zero.
        assert!(research_identifier_normalize(IdentifierSystem::Ror, "12mhbdp94").is_none());
    }

    #[test]
    fn pubmed_normalization_is_canonical() {
        assert_eq!(
            research_identifier_normalize(IdentifierSystem::PubMed, "PMID:24239612"),
            Some("24239612".to_owned())
        );
        assert_eq!(
            research_identifier_normalize(IdentifierSystem::PubMed, "0024239"),
            Some("24239".to_owned())
        );
        assert!(research_identifier_normalize(IdentifierSystem::PubMed, "24a239").is_none());
    }

    #[test]
    fn normalization_is_deterministic_across_repeated_calls() {
        let inputs = [
            (IdentifierSystem::Doi, "https://doi.org/10.1000/ABC"),
            (IdentifierSystem::Orcid, "0000-0002-1825-0097"),
            (IdentifierSystem::Issn, "03785955"),
        ];
        for (system, value) in inputs {
            let first = research_identifier_normalize(system, value);
            for _ in 0..5 {
                assert_eq!(research_identifier_normalize(system, value), first);
            }
        }
    }

    #[test]
    fn unknown_system_identifiers_are_rejected() {
        assert!(IdentifierSystem::from_identifier("scopus").is_none());
        assert_eq!(
            IdentifierSystem::from_identifier("orcid"),
            Some(IdentifierSystem::Orcid)
        );
    }
}
