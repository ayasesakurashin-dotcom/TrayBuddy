use super::providers;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const ROUTER_VERSION: &str = "web_search_router_v2";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteAction {
    Direct,
    Search,
    Clarify,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteReason {
    ExplicitSearch,
    TimeSensitive,
    SpecificFact,
    SourceCheck,
    StableCommon,
    PersonalMemory,
    Emotional,
    Creative,
    ProvidedText,
    NoWeb,
    MissingContext,
    MultipleTopics,
    Weather,
    LocalTime,
    Uncertain,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub action: RouteAction,
    pub reason: RouteReason,
    pub query: String,
    pub clarification: String,
}

pub fn validate_route(route: &Route) -> bool {
    let search_reason = matches!(
        route.reason,
        RouteReason::ExplicitSearch
            | RouteReason::TimeSensitive
            | RouteReason::SpecificFact
            | RouteReason::SourceCheck
    );
    let clarify_reason = matches!(
        route.reason,
        RouteReason::MissingContext | RouteReason::MultipleTopics
    );
    match route.action {
        RouteAction::Search => {
            search_reason
                && (1..=128).contains(&route.query.chars().count())
                && !route.query.trim().is_empty()
                && route.clarification.is_empty()
        }
        RouteAction::Clarify => {
            clarify_reason
                && route.query.is_empty()
                && (1..=80).contains(&route.clarification.chars().count())
                && !route.clarification.trim().is_empty()
        }
        RouteAction::Direct => {
            !search_reason
                && !clarify_reason
                && route.query.is_empty()
                && route.clarification.is_empty()
        }
    }
}

/// Deliberately conservative guard, supplementary to semantic classification.
/// Quoted text, fenced code and block quotes cannot issue networking commands.
pub fn unquoted_input(text: &str) -> String {
    let mut output = String::new();
    let mut fenced = false;
    let mut closing = None;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced || line.trim_start().starts_with('>') {
            continue;
        }
        for c in line.chars() {
            if closing == Some(c) {
                closing = None;
                continue;
            }
            if closing.is_some() {
                continue;
            }
            closing = match c {
                '“' => Some('”'),
                '「' => Some('」'),
                '『' => Some('』'),
                '"' => Some('"'),
                '`' => Some('`'),
                _ => None,
            };
            if closing.is_none() {
                output.push(c);
            }
        }
        output.push('\n');
    }
    output.to_lowercase()
}
pub fn forbids_web(text: &str) -> bool {
    let input = unquoted_input(text);
    [
        "不要联网",
        "不许联网",
        "别联网",
        "不要上网",
        "不用上网",
        "不要搜索",
        "不用搜索",
        "别搜索",
        "不联网",
        "离线回答",
        "do not search",
        "don't search",
        "do not browse",
        "don't browse",
        "without browsing",
        "no web search",
    ]
    .iter()
    .any(|phrase| input.contains(phrase))
}
pub fn explicitly_requests_verification(text: &str) -> bool {
    if forbids_web(text) {
        return false;
    }
    let input = unquoted_input(text);
    [
        "搜索",
        "上网查",
        "联网查",
        "查一下",
        "查一查",
        "查查",
        "查找",
        "核实",
        "核查",
        "查证",
        "出处",
        "官方依据",
        "最新",
        "search",
        "browse",
        "verify",
        "source",
        "look up",
    ]
    .iter()
    .any(|phrase| input.contains(phrase))
}

pub fn safe_query(query: &str) -> Option<String> {
    let query = query.trim();
    if !(1..=128).contains(&query.chars().count())
        || providers::redact(query) != query
        || query.chars().any(char::is_control)
    {
        return None;
    }
    static PRIVATE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let private = PRIVATE.get_or_init(|| regex::Regex::new(r"(?i)(?:[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}|(?:\+?86[- ]?)?1[3-9]\d{9}|\+\d[\d ()-]{7,}\d|[a-z]:[\\/]|\\\\[^\s]+|/(?:users|home|private|tmp)/|\b(?:localhost|127\.\d+\.\d+\.\d+|10\.\d+\.\d+\.\d+|192\.168\.\d+\.\d+|172\.(?:1[6-9]|2\d|3[01])\.\d+\.\d+)\b|\b(?:token|secret|authorization|access_key|session_id)\s*[:=]|https?://[^\s]+[?#])").unwrap());
    if private.is_match(query) {
        return None;
    }
    Some(query.to_string())
}

/// Only browser-navigable public domain URLs; no requests are made here.
pub fn public_url(raw: &str) -> Option<reqwest::Url> {
    if raw.len() > 4096 || raw.chars().any(char::is_control) {
        return None;
    }
    let url = reqwest::Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    if host
        .trim_matches(['[', ']'])
        .parse::<std::net::IpAddr>()
        .is_ok()
    {
        return None;
    }
    if !host.contains('.') || host.ends_with('.') && !host.trim_end_matches('.').contains('.') {
        return None;
    }
    let host = host.trim_end_matches('.');
    if [
        "localhost",
        "local",
        "internal",
        "lan",
        "home",
        "test",
        "invalid",
    ]
    .iter()
    .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
    {
        return None;
    }
    Some(url)
}

fn canonical_url(url: &reqwest::Url) -> String {
    let mut canonical = url.clone();
    canonical.set_fragment(None);
    let pairs: Vec<(String, String)> = canonical
        .query_pairs()
        .filter(|(key, _)| {
            !key.to_ascii_lowercase().starts_with("utm_")
                && !matches!(
                    key.to_ascii_lowercase().as_str(),
                    "fbclid" | "gclid" | "msclkid"
                )
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    canonical.set_query(None);
    if !pairs.is_empty() {
        canonical.query_pairs_mut().extend_pairs(pairs);
    }
    canonical.into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WebSource {
    pub id: String,
    pub title: String,
    pub url: String,
    pub date: Option<String>,
    pub retrieved_at: String,
    pub evidence_kind: String,
    pub excerpt: String,
}

#[derive(Deserialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
}
#[derive(Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub snippet: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub authority: Option<i32>,
    #[serde(default)]
    pub rerank_score: Option<f64>,
}

fn query_terms(query: &str) -> Vec<String> {
    let lower = query.to_lowercase();
    let mut terms = Vec::new();
    for word in lower.split(|c: char| !c.is_alphanumeric()) {
        if word.is_empty()
            || [
                "官方",
                "公告",
                "原文",
                "最新",
                "是什么",
                "有哪些",
                "查询",
                "搜索",
                "official",
                "the",
                "and",
            ]
            .contains(&word)
        {
            continue;
        }
        if word.chars().count() > 4 && word.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
        {
            let chars: Vec<char> = word.chars().collect();
            for pair in chars.windows(2) {
                terms.push(pair.iter().collect());
            }
        } else if word.chars().count() >= 2 {
            terms.push(word.into());
        }
    }
    terms.sort();
    terms.dedup();
    terms
}

fn relevance(text: &str, terms: &[String]) -> usize {
    let lower = text.to_lowercase();
    terms
        .iter()
        .filter(|term| lower.contains(term.as_str()))
        .count()
}

fn clean_text(text: &str) -> String {
    // Content remains plain data even if a provider returns HTML.
    static TAGS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let tags = TAGS.get_or_init(|| {
        regex::Regex::new(r"(?is)<(?:script|style)\b[^>]*>.*?</(?:script|style)>|<[^>]{0,500}>")
            .unwrap()
    });
    tags.replace_all(text, " ")
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}

/// Select whole sentences around matching terms, maintaining source order and negation.
fn excerpt(text: &str, terms: &[String], max_chars: usize) -> String {
    let cleaned = clean_text(text);
    let sentences: Vec<&str> = cleaned
        .split_inclusive(['。', '！', '？', '\n', '.', '!', '?'])
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .collect();
    let mut ranked: Vec<(usize, usize)> = sentences
        .iter()
        .enumerate()
        .map(|(index, s)| (index, relevance(s, terms)))
        .filter(|(_, score)| *score > 0)
        .collect();
    ranked.sort_by_key(|(index, score)| (std::cmp::Reverse(*score), *index));
    let mut selected = std::collections::BTreeSet::new();
    let mut chars = 0;
    for (index, _) in ranked {
        if sentences[index].chars().count() > max_chars.saturating_sub(chars + selected.len()) {
            continue;
        }
        for candidate in [
            index,
            index.saturating_sub(1),
            (index + 1).min(sentences.len().saturating_sub(1)),
        ] {
            let count = sentences[candidate].chars().count();
            if !selected.contains(&candidate)
                && count <= max_chars.saturating_sub(chars + selected.len())
            {
                selected.insert(candidate);
                chars += count;
            }
        }
    }
    selected
        .into_iter()
        .map(|index| sentences[index])
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn select_sources(response: SearchResponse, query: &str, retrieved_at: &str) -> Vec<WebSource> {
    let terms = query_terms(query);
    if terms.is_empty() {
        return vec![];
    }
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    for item in response.results {
        let Some(url) = public_url(&item.url) else {
            continue;
        };
        if !seen.insert(canonical_url(&url)) {
            continue;
        }
        let (text, kind) =
            if let Some(content) = item.content.as_deref().filter(|s| !s.trim().is_empty()) {
                (content, "page_excerpt")
            } else if let Some(summary) = item.summary.as_deref().filter(|s| !s.trim().is_empty()) {
                (summary, "service_summary")
            } else if let Some(snippet) = item.snippet.as_deref().filter(|s| !s.trim().is_empty()) {
                (snippet, "service_summary")
            } else {
                continue;
            };
        let score = relevance(text, &terms);
        if score == 0 {
            continue;
        }
        let excerpt = excerpt(text, &terms, 1800);
        if excerpt.is_empty() {
            continue;
        }
        let date = item
            .date
            .filter(|d| chrono::DateTime::parse_from_rfc3339(d).is_ok());
        // Direct evidence relevance ranks first; service quality is only a tie-breaker.
        candidates.push((
            score,
            kind == "page_excerpt",
            item.authority.unwrap_or(-1),
            item.rerank_score.unwrap_or(0.0),
            WebSource {
                id: String::new(),
                title: clean_text(&item.title).chars().take(200).collect(),
                url: item.url,
                date,
                retrieved_at: retrieved_at.into(),
                evidence_kind: kind.into(),
                excerpt,
            },
        ));
    }
    candidates.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.cmp(&a.1))
            .then(b.2.cmp(&a.2))
            .then(b.3.total_cmp(&a.3))
    });
    let mut sources = Vec::new();
    let mut total = 0;
    for (_, _, _, _, mut source) in candidates {
        source.id = format!("s{}", sources.len() + 1);
        let size = serde_json::to_string(&source)
            .unwrap_or_default()
            .chars()
            .count();
        if total + size > 6200 {
            continue;
        }
        total += size;
        sources.push(source);
        if sources.len() == 3 {
            break;
        }
    }
    sources
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn route_contract_rejects_unknown_truncated_and_crossed_fields() {
        let direct: Route = serde_json::from_value(
            json!({"action":"direct","reason":"uncertain","query":"","clarification":""}),
        )
        .unwrap();
        assert!(validate_route(&direct));
        for value in [
            json!({"action":"search","reason":"uncertain","query":"hello","clarification":""}),
            json!({"action":"direct","reason":"specific_fact","query":"","clarification":""}),
            json!({"action":"clarify","reason":"missing_context","query":"x","clarification":"哪一个？"}),
            json!({"action":"search","reason":"source_check","query":"🙂".repeat(129),"clarification":""}),
        ] {
            assert!(!validate_route(&serde_json::from_value(value).unwrap()));
        }
        assert!(serde_json::from_value::<Route>(json!({"action":"direct","reason":"uncertain","query":"","clarification":"","confidence":0.9})).is_err());
        assert!(serde_json::from_value::<Route>(
            json!({"action":"direct","reason":"uncertain","query":""})
        )
        .is_err());
        assert!(validate_route(&Route {
            action: RouteAction::Search,
            reason: RouteReason::SpecificFact,
            query: "🙂".repeat(128),
            clarification: String::new()
        }));
    }
    #[test]
    fn networking_controls_ignore_quoted_instructions() {
        assert!(forbids_web("不要联网，核实最新消息"));
        assert!(!explicitly_requests_verification("不要联网，核实最新消息"));
        assert!(!forbids_web("翻译“不要联网”"));
        assert!(!forbids_web("解释：\n```text\n不要联网\n```"));
        assert!(!explicitly_requests_verification("翻译“请搜索最新消息”"));
        assert!(explicitly_requests_verification("帮我核实活动是否延期"));
    }
    #[test]
    fn query_privacy_does_not_destroy_public_versions_or_dates() {
        for query in [
            "原神 5.0 世界等级上限 2024 官方公告",
            "上海博物馆 2026 开放时间",
        ] {
            assert_eq!(safe_query(query).as_deref(), Some(query));
        }
        for query in [
            "mail@example.com",
            "13800138000",
            "C:\\Users\\test",
            "10.0.0.1",
            "http://localhost/",
            "https://example.com/?token=abc",
            "api_key: sk-abcdefghijklmnop",
        ] {
            assert!(safe_query(query).is_none(), "{query}");
        }
    }
    #[test]
    fn public_source_validation_rejects_private_navigation() {
        for url in [
            "file:///C:/test",
            "javascript:alert(1)",
            "https://127.0.0.1/a",
            "http://2130706433/",
            "http://[::1]/",
            "https://user:pass@example.com/",
            "http://example.local",
            "https://localhost./",
            "http://host/",
        ] {
            assert!(public_url(url).is_none(), "{url}");
        }
        assert!(public_url("https://www.python.org/downloads/?version=3.14#abc").is_some());
    }
    #[test]
    fn evidence_selection_uses_relevant_complete_sentences_and_actual_source_kind() {
        let response = serde_json::from_value(json!({"results":[
            {"title":"公告","url":"https://example.com/page?utm_source=test","content":format!("{}\n产品A 5.0 不支持离线模式。这个限制仅适用于5.0。", "无关开场。".repeat(400)),"date":null},
            {"title":"重复","url":"https://example.com/page#other","content":"产品A 5.0 支持离线。"},
            {"title":"摘要","url":"https://example.org/item","summary":"产品A 5.0 可能在下月更新。","date":"bad-date"},
            {"title":"无资料","url":"https://example.net/item","content":"完全无关。"}
        ]})).unwrap();
        let sources = select_sources(response, "产品A 5.0 离线模式", "2026-10-22T00:00:00Z");
        assert_eq!(sources.len(), 2);
        assert!(sources[0].excerpt.contains("不支持离线模式"));
        assert!(sources[0].excerpt.chars().count() <= 1800);
        assert_eq!(sources[0].evidence_kind, "page_excerpt");
        assert_eq!(sources[1].evidence_kind, "service_summary");
        assert!(sources[1].date.is_none());
        assert!(serde_json::to_string(&sources).unwrap().chars().count() <= 6500);
    }
    #[test]
    fn protocol_errors_and_empty_evidence_are_not_success() {
        assert!(serde_json::from_value::<SearchResponse>(
            json!({"results":[{"title":2,"url":"https://example.com"}]})
        )
        .is_err());
        assert!(serde_json::from_value::<SearchResponse>(
            json!({"results":[{"title":"A","url":"https://example.com","content":[]}]})
        )
        .is_err());
        assert!(select_sources(
            serde_json::from_value(json!({"results":[]})).unwrap(),
            "Python 版本",
            "now"
        )
        .is_empty());
        assert!(select_sources(
            serde_json::from_value(
                json!({"results":[{"title":"Python","url":"https://example.com","content":""}]})
            )
            .unwrap(),
            "Python 版本",
            "now"
        )
        .is_empty());
    }
}
