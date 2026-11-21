//! Duplicate market proposal detection and deterministic ranking.

use std::collections::{BTreeSet, HashSet};

use chrono::{DateTime, Utc};

pub const MAX_CANDIDATES: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub proposal_id: String,
    pub question: String,
    pub event_window_start: DateTime<Utc>,
    pub event_window_end: DateTime<Utc>,
    pub source: String,
    pub outcomes: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct Candidate {
    pub proposal_id: String,
    pub question: String,
    pub event_window_start: DateTime<Utc>,
    pub event_window_end: DateTime<Utc>,
    pub source: String,
    pub outcomes: Vec<String>,
    pub score: u8,
}

/// Normalize punctuation and whitespace without transliterating scripts.
pub fn normalize_text(input: &str) -> String {
    input
        .chars()
        .flat_map(char::to_lowercase)
        .map(|ch| {
            if ch.is_alphanumeric()
                || matches!(ch as u32, 0x0300..=0x036f | 0x1ab0..=0x1aff | 0x1dc0..=0x1dff)
            {
                ch
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn script_family(text: &str) -> BTreeSet<&'static str> {
    text.chars()
        .filter(|ch| ch.is_alphabetic())
        .filter_map(|ch| {
            let code = ch as u32;
            match code {
                0x0041..=0x024f | 0x1e00..=0x1eff | 0xab30..=0xab6f => Some("latin"),
                0x0370..=0x03ff | 0x1f00..=0x1fff => Some("greek"),
                0x0400..=0x052f | 0x2de0..=0x2dff | 0xa640..=0xa69f => Some("cyrillic"),
                0x0590..=0x05ff => Some("hebrew"),
                0x0600..=0x08ff => Some("arabic"),
                0x0900..=0x097f => Some("devanagari"),
                0x3040..=0x30ff => Some("kana"),
                0x3400..=0x9fff => Some("han"),
                0xac00..=0xd7af => Some("hangul"),
                _ => Some("other"),
            }
        })
        .collect()
}

fn tokens(text: &str) -> HashSet<String> {
    let normalized = normalize_text(text);
    if normalized.chars().any(|ch| ch.is_whitespace()) {
        normalized.split_whitespace().map(str::to_string).collect()
    } else if script_family(&normalized).iter().any(|s| *s != "latin") {
        normalized.chars().map(|ch| ch.to_string()).collect()
    } else if normalized.is_empty() {
        HashSet::new()
    } else {
        [normalized].into_iter().collect()
    }
}

fn question_similarity(left: &str, right: &str) -> f64 {
    if script_family(left) != script_family(right) {
        return 0.0;
    }
    let left = tokens(left);
    let right = tokens(right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(&right).count();
    let union = left.union(&right).count();
    intersection as f64 / union as f64
}

fn normalized_outcomes(outcomes: &[String]) -> BTreeSet<String> {
    outcomes
        .iter()
        .map(|outcome| normalize_text(outcome))
        .collect()
}

fn window_similarity(left: &Proposal, right: &Proposal) -> f64 {
    let left_start = left.event_window_start.timestamp();
    let left_end = left.event_window_end.timestamp();
    let right_start = right.event_window_start.timestamp();
    let right_end = right.event_window_end.timestamp();
    let overlap = left_end
        .min(right_end)
        .saturating_sub(left_start.max(right_start));
    if overlap < 0 {
        return 0.0;
    }
    let longer = (left_end - left_start).max(right_end - right_start);
    if longer == 0 {
        return if left_start == right_start { 1.0 } else { 0.0 };
    }
    overlap as f64 / longer as f64
}

fn normalized_source(source: &str) -> String {
    source.trim().trim_end_matches('/').to_lowercase()
}

/// Rank only proposals whose source and outcome set match, whose event windows
/// substantially overlap, and whose question text is sufficiently similar.
pub fn rank_candidates(proposal: &Proposal, candidates: &[Proposal]) -> Vec<Candidate> {
    let mut ranked = candidates
        .iter()
        .filter(|candidate| candidate.proposal_id != proposal.proposal_id)
        .filter_map(|candidate| {
            let text_score = question_similarity(&proposal.question, &candidate.question);
            let window_score = window_similarity(proposal, candidate);
            let source_matches =
                normalized_source(&proposal.source) == normalized_source(&candidate.source);
            let outcomes_match =
                normalized_outcomes(&proposal.outcomes) == normalized_outcomes(&candidate.outcomes);

            if text_score < 0.60 || window_score < 0.50 || !source_matches || !outcomes_match {
                return None;
            }

            let score = ((text_score * 60.0)
                + (window_score * 20.0)
                + if source_matches { 10.0 } else { 0.0 }
                + if outcomes_match { 10.0 } else { 0.0 })
            .round() as u8;
            Some(Candidate {
                proposal_id: candidate.proposal_id.clone(),
                question: candidate.question.clone(),
                event_window_start: candidate.event_window_start,
                event_window_end: candidate.event_window_end,
                source: candidate.source.clone(),
                outcomes: candidate.outcomes.clone(),
                score,
            })
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.proposal_id.cmp(&right.proposal_id))
    });
    ranked.truncate(MAX_CANDIDATES);
    ranked
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::{normalize_text, rank_candidates, Proposal};

    fn proposal(id: &str, question: &str, day: i64) -> Proposal {
        let start = chrono::Utc.timestamp_opt(day * 86_400, 0).unwrap();
        Proposal {
            proposal_id: id.into(),
            question: question.into(),
            event_window_start: start,
            event_window_end: start + Duration::days(4),
            source: "https://example.com/results".into(),
            outcomes: vec!["Yes".into(), "No".into()],
        }
    }

    #[test]
    fn normalizes_case_punctuation_and_unicode_whitespace() {
        assert_eq!(
            normalize_text("  ¿Will   BTC—hit $100k?  "),
            "will btc hit 100k"
        );
        assert_eq!(normalize_text("¿Ganará España?"), "ganará españa");
    }

    #[test]
    fn multilingual_scripts_do_not_match_as_false_positives() {
        let input = proposal("new", "Will the market rise?", 100);
        let translated = proposal("old", "¿Subirá el mercado?", 100);
        assert!(rank_candidates(&input, &[translated]).is_empty());
    }

    #[test]
    fn ranks_deterministically_and_keeps_distinct_markets_available() {
        let input = proposal("new", "Will BTC reach $100k in 2026?", 100);
        let exact = proposal("b", "Will BTC reach 100k in 2026", 100);
        let near = proposal("a", "BTC reach 100k in 2026?", 100);
        let other_source = Proposal {
            source: "https://other.example.com".into(),
            ..proposal("c", "Will BTC reach 100k in 2026?", 100)
        };
        let other_window = proposal("d", "Will BTC reach 100k in 2026?", 200);
        let candidates = rank_candidates(&input, &[near, other_source, exact, other_window]);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.proposal_id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "a"]
        );
        assert!(candidates[0].score >= candidates[1].score);
    }

    #[test]
    fn requires_matching_outcomes_and_substantial_window_overlap() {
        let input = proposal("new", "Will BTC reach $100k in 2026?", 100);
        let different_outcomes = Proposal {
            outcomes: vec!["Above $100k".into(), "Below $100k".into()],
            ..proposal("outcomes", "Will BTC reach 100k in 2026?", 100)
        };
        let partial_window = proposal("window", "Will BTC reach 100k in 2026?", 103);
        assert!(rank_candidates(&input, &[different_outcomes, partial_window]).is_empty());
    }

    #[test]
    fn breaks_equal_score_ties_by_stable_id() {
        let input = proposal("new", "Will BTC reach $100k in 2026?", 100);
        let first = proposal("a-stable-id", "Will BTC reach 100k in 2026?", 100);
        let second = proposal("b-stable-id", "Will BTC reach 100k in 2026?", 100);
        let candidates = rank_candidates(&input, &[second, first]);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.proposal_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a-stable-id", "b-stable-id"]
        );
    }
}
