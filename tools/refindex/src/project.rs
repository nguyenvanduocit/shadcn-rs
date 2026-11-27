//! This project's own vocabulary, read from the repository at index time:
//! ADRs (with status and supersession), ledger tasks, failure classes, decisions and issues,
//! and peer bugs our docs cite (`Leptos #4457`). These become graph nodes ("entities") that
//! chunks mention, so one hop links an ADR or failure class to the peer code and tests behind it.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;

use crate::config;

#[derive(Debug, Clone)]
pub struct Vocab {
    /// Ledger task ids (K4b, SLICE, ...) -> title.
    pub tasks: HashMap<String, (String, String)>,
    /// Peer repo names, lowercase.
    pub peers: HashSet<String>,
    task_rx: Option<Regex>,
    peer_bug_rx: Option<Regex>,
}

static ADR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bADR[- ]?0*(\d{1,4})\b|docs/adr/(\d{4})-").unwrap());
static CLASS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bfailure[- ]class(?:es)?\s*#?(\d{1,2})\b|\bclass\s+#?(\d{1,2})\b").unwrap());
static DECISION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bD(\d{3})\b").unwrap());
static ISSUE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:issue|bug|gh|regression)_?(\d{3,})").unwrap());

impl Vocab {
    pub fn load(peers: &HashSet<String>) -> Self {
        let mut tasks = HashMap::new();
        let ledger = config::root().join(".claude/guard/ledger.json");
        if let Ok(s) = std::fs::read_to_string(ledger) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                if let Some(t) = v.get("tasks").and_then(|t| t.as_object()) {
                    for (id, t) in t {
                        let title = t.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string();
                        let status = t.get("status").and_then(|x| x.as_str()).unwrap_or("").to_string();
                        tasks.insert(id.clone(), (title, status));
                    }
                }
            }
        }
        let mut ids: Vec<&String> = tasks.keys().collect();
        ids.sort_by_key(|k| std::cmp::Reverse(k.len()));
        let task_rx = (!ids.is_empty()).then(|| {
            Regex::new(&format!(r"\b({})\b", ids.iter().map(|k| regex::escape(k)).collect::<Vec<_>>().join("|")))
                .unwrap()
        });
        let peers_lc: HashSet<String> = peers.iter().map(|p| p.to_lowercase()).collect();
        let mut names: Vec<&String> = peers_lc.iter().collect();
        names.sort_by_key(|k| std::cmp::Reverse(k.len()));
        let peer_bug_rx = (!names.is_empty()).then(|| {
            Regex::new(&format!(r"(?i)\b({})\s*#(\d{{2,}})",
                                names.iter().map(|k| regex::escape(k)).collect::<Vec<_>>().join("|"))).unwrap()
        });
        Vocab { tasks, peers: peers_lc, task_rx, peer_bug_rx }
    }

    /// Entities a chunk mentions. Project entities only count in our own docs; peer bugs count
    /// everywhere (a peer test named `issue_4457` mentions `leptos#4457`).
    pub fn mentions(&self, repo: &str, name: &str, text: &str, bugs: &[String]) -> Vec<String> {
        let mut out: HashSet<String> = HashSet::new();
        let own = repo.starts_with('@');
        if own {
            for c in ADR.captures_iter(text) {
                let n: u32 = c.get(1).or(c.get(2)).unwrap().as_str().parse().unwrap_or(0);
                out.insert(format!("adr:{n:04}"));
            }
            for c in CLASS.captures_iter(text) {
                out.insert(format!("class:{}", c.get(1).or(c.get(2)).unwrap().as_str()));
            }
            for c in DECISION.captures_iter(text) {
                out.insert(format!("decision:D{}", &c[1]));
            }
            if let Some(rx) = &self.task_rx {
                for c in rx.captures_iter(text) {
                    out.insert(format!("task:{}", &c[1]));
                }
            }
            if let Some(rx) = &self.peer_bug_rx {
                for c in rx.captures_iter(text) {
                    out.insert(format!("peerbug:{}#{}", c[1].to_lowercase(), &c[2]));
                }
            }
        } else {
            let me = repo.to_lowercase();
            for b in bugs {
                if let Some(n) = b.strip_prefix("bug:") {
                    out.insert(format!("peerbug:{me}#{n}"));
                } else if let Some(rest) = b.strip_prefix("gh:") {
                    // gh:owner/name#N -> peerbug:name#N when name is a peer we index
                    if let Some((slug, n)) = rest.split_once('#') {
                        let name = slug.rsplit('/').next().unwrap_or("").to_lowercase();
                        let name = if self.peers.contains(&name) { name } else { me.clone() };
                        out.insert(format!("peerbug:{name}#{n}"));
                    }
                }
            }
            for c in ISSUE_NAME.captures_iter(name) {
                out.insert(format!("peerbug:{me}#{}", &c[1]));
            }
        }
        let mut v: Vec<String> = out.into_iter().collect();
        v.sort();
        v
    }
}

/// Normalise what a user types into an entity key: `ADR-0026`, `adr:26`, `K4b`, `task:K4b`,
/// `class 3`, `D012`, `Leptos #4457`.
pub fn parse_entity(s: &str, vocab: &Vocab) -> String {
    let t = s.trim();
    if let Some(c) = ADR.captures(t) {
        let n: u32 = c.get(1).or(c.get(2)).unwrap().as_str().parse().unwrap_or(0);
        return format!("adr:{n:04}");
    }
    if let Some(n) = t.strip_prefix("adr:") {
        return format!("adr:{:04}", n.parse::<u32>().unwrap_or(0));
    }
    if let Some(c) = CLASS.captures(t) {
        return format!("class:{}", c.get(1).or(c.get(2)).unwrap().as_str());
    }
    if let Some(c) = DECISION.captures(t) {
        return format!("decision:D{}", &c[1]);
    }
    if vocab.tasks.contains_key(t) {
        return format!("task:{t}");
    }
    if let Some(rx) = &vocab.peer_bug_rx {
        if let Some(c) = rx.captures(t) {
            return format!("peerbug:{}#{}", c[1].to_lowercase(), &c[2]);
        }
    }
    t.to_string()
}

/// ADR number of a docs path `adr/0026-....md`.
pub fn adr_of_path(path: &str) -> Option<String> {
    let f = path.strip_prefix("adr/")?;
    let n: u32 = f.get(..4)?.parse().ok()?;
    Some(format!("adr:{n:04}"))
}

/// Status of an ADR file and its supersession edges as (superseding, superseded) pairs relative
/// to `me`.
pub fn adr_meta(me: &str, text: &str) -> (String, Vec<(String, String)>) {
    static STATUS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?im)^\s*\**status\**:?\**\s*(.+)$").unwrap());
    static SUPERSEDES: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\bsupersedes(?:,? in part,)?[^.\n]{0,80}?ADR-0*(\d{1,4})").unwrap());
    static SUPERSEDED_BY: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)superseded(?: in part)? by[^.\n]{0,40}?ADR-0*(\d{1,4})").unwrap());
    let adr = |c: &regex::Captures| format!("adr:{:04}", c[1].parse::<u32>().unwrap_or(0));
    let status = STATUS.captures(text).map(|c| c[1].trim().to_string()).unwrap_or_default();
    let mut links: Vec<(String, String)> = SUPERSEDES.captures_iter(text).map(|c| (me.to_string(), adr(&c))).collect();
    links.extend(SUPERSEDED_BY.captures_iter(text).map(|c| (adr(&c), me.to_string())));
    links.retain(|(a, b)| a != b);
    links.sort();
    links.dedup();
    (status, links)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab() -> Vocab {
        let peers: HashSet<String> = ["leptos", "chromium"].iter().map(|s| s.to_string()).collect();
        let mut v = Vocab::load(&peers);
        v.tasks.insert("K4b".into(), ("bound attributes".into(), "done".into()));
        v.task_rx = Some(Regex::new(r"\b(K4b)\b").unwrap());
        v
    }

    #[test]
    fn own_docs_mention_project_entities() {
        let v = vocab();
        let m = v.mentions("@docs", "", "K4b broke this, see ADR-0026, failure class 3 and Leptos #4457; D012.", &[]);
        for e in ["task:K4b", "adr:0026", "class:3", "peerbug:leptos#4457", "decision:D012"] {
            assert!(m.contains(&e.to_string()), "{e} not in {m:?}");
        }
    }

    #[test]
    fn peer_code_mentions_its_bugs() {
        let v = vocab();
        let m = v.mentions("leptos", "issue_4457_effect_runs_after_dispose", "", &["bug:4500".into(),
            "gh:leptos-rs/leptos#4457".into()]);
        assert_eq!(m, vec!["peerbug:leptos#4457", "peerbug:leptos#4500"]);
        assert!(v.mentions("leptos", "", "see ADR-0026", &[]).is_empty());
    }

    #[test]
    fn adr_status_and_supersession() {
        let (s, sup) = adr_meta("adr:0026", "# ADR\n\nStatus: accepted, 2026-11-24. Supersedes ADR-0013's instance.\n");
        assert!(s.starts_with("accepted"));
        assert_eq!(sup, vec![("adr:0026".to_string(), "adr:0013".to_string())]);
        let (_, by) = adr_meta("adr:0006", "Status: superseded by ADR-0014\n");
        assert_eq!(by, vec![("adr:0014".to_string(), "adr:0006".to_string())]);
        assert_eq!(parse_entity("ADR-26", &vocab()), "adr:0026");
        assert_eq!(parse_entity("Leptos #4457", &vocab()), "peerbug:leptos#4457");
    }
}
