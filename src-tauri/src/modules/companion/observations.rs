//! Role-independent wire contracts and validated screen evidence (CTX-OBS-1).
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScreenSelection {
    #[default]
    None,
    Fresh {
        target_token: String,
    },
    Reference {
        observation_ids: Vec<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatSendPayload {
    pub client_message_id: String,
    pub expected_history_epoch: u64,
    pub text: String,
    pub screen: ScreenSelection,
}

impl ChatSendPayload {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_uuid(&self.client_message_id) {
            return Err("request_id_invalid".into());
        }
        if self.text.trim().is_empty() || self.text.chars().count() > 4_000 {
            return Err("message_invalid".into());
        }
        match &self.screen {
            ScreenSelection::Fresh { target_token } if !valid_uuid(target_token) => {
                Err("screen_target_unavailable".into())
            }
            ScreenSelection::Reference { observation_ids } => validate_ids(observation_ids),
            _ => Ok(()),
        }
    }
}

pub fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
        && value.as_bytes()[14] == b'4'
        && matches!(
            value.as_bytes()[19].to_ascii_lowercase(),
            b'8' | b'9' | b'a' | b'b'
        )
        && value == value.to_ascii_lowercase()
}

pub fn validate_ids(ids: &[String]) -> Result<(), String> {
    if ids.is_empty()
        || ids.len() > 3
        || ids.iter().any(|id| id.is_empty() || id.len() > 100)
        || ids.iter().collect::<std::collections::HashSet<_>>().len() != ids.len()
    {
        return Err("observation_reference_invalid".into());
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SendOutcome {
    pub request: Option<u64>,
    pub turn_id: Option<String>,
    pub result: String,
    pub text_committed: bool,
    pub observation_ids: Vec<String>,
    pub history_epoch: u64,
    pub replayed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning_code: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestReceipt {
    pub client_message_id: String,
    pub history_epoch: u64,
    pub state: String,
    pub request: Option<u64>,
    pub turn_id: Option<String>,
    pub text_committed: bool,
    pub outcome: Option<SendOutcome>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreparedScreenTarget {
    pub target_token: String,
    pub label: String,
    pub scope: String,
    pub expires_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisibleFact {
    pub text: String,
    pub basis: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisualEvidence {
    pub usable: bool,
    pub sensitive: bool,
    pub confidence: f64,
    pub answerability: String,
    pub summary: String,
    pub visible_text: String,
    pub facts: Vec<VisibleFact>,
    pub uncertainties: Vec<String>,
}

impl VisualEvidence {
    pub fn parse(raw: &str, question: bool) -> Result<Self, String> {
        if raw.len() > 100_000 {
            return Err("screen_vision_invalid".into());
        }
        let raw = raw.trim();
        let raw = raw
            .strip_prefix("```json")
            .or_else(|| raw.strip_prefix("```"))
            .and_then(|s| s.strip_suffix("```"))
            .unwrap_or(raw)
            .trim();
        let mut value: serde_json::Value =
            serde_json::from_str(raw).map_err(|_| "screen_vision_invalid")?;
        if !question {
            let object = value.as_object_mut().ok_or("screen_vision_invalid")?;
            object
                .entry("answerability")
                .or_insert(serde_json::json!("sufficient"));
            object
                .entry("visible_text")
                .or_insert(serde_json::json!(""));
            object.entry("facts").or_insert(serde_json::json!([]));
            object
                .entry("uncertainties")
                .or_insert(serde_json::json!([]));
        }
        let evidence: Self = serde_json::from_value(value).map_err(|_| "screen_vision_invalid")?;
        evidence.validate()?;
        Ok(evidence)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.sensitive {
            return Err("screen_sensitive".into());
        }
        if !self.confidence.is_finite()
            || !(0.0..=1.0).contains(&self.confidence)
            || !matches!(
                self.answerability.as_str(),
                "sufficient" | "partial" | "insufficient"
            )
            || self.summary.chars().count() > 500
            || self.visible_text.chars().count() > 2_000
            || self.facts.len() > 8
            || self.uncertainties.len() > 4
            || self
                .facts
                .iter()
                .any(|f| f.text.chars().count() + f.basis.chars().count() > 300)
            || self.uncertainties.iter().any(|s| s.chars().count() > 150)
        {
            return Err("screen_vision_invalid".into());
        }
        let total = self.summary.chars().count()
            + self.visible_text.chars().count()
            + self
                .facts
                .iter()
                .map(|f| f.text.chars().count() + f.basis.chars().count())
                .sum::<usize>()
            + self
                .uncertainties
                .iter()
                .map(|s| s.chars().count())
                .sum::<usize>();
        if total > 6_144 {
            return Err("screen_vision_invalid".into());
        }
        if !self.usable
            || self.confidence < 0.6
            || self.answerability == "insufficient"
            || self.summary.trim().is_empty()
        {
            return Err("screen_unusable".into());
        }
        Ok(())
    }
    pub fn redact(mut self, redact: impl Fn(&str) -> String) -> Self {
        self.summary = redact(&self.summary);
        self.visible_text = redact(&self.visible_text);
        for fact in &mut self.facts {
            fact.text = redact(&fact.text);
            fact.basis = redact(&fact.basis);
        }
        self.uncertainties = self.uncertainties.iter().map(|s| redact(s)).collect();
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObservationDraft {
    pub id: String,
    pub origin: String,
    pub observed_at: String,
    pub scope: String,
    pub target_label: String,
    pub evidence: VisualEvidence,
}
impl ObservationDraft {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_uuid(&self.id)
            || !matches!(
                self.origin.as_str(),
                "automatic" | "manual_once" | "attached_question"
            )
            || !matches!(self.scope.as_str(), "foreground" | "screen" | "region")
            || self.target_label.chars().count() > 200
            || chrono::DateTime::parse_from_rfc3339(&self.observed_at).is_err()
        {
            return Err("screen_vision_invalid".into());
        }
        self.evidence.validate()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub id: String,
    pub created_by_turn_id: String,
    pub origin: String,
    pub observed_at: String,
    pub time_basis: String,
    pub scope: String,
    pub target_label: String,
    pub summary: String,
    pub evidence: Option<VisualEvidence>,
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy_original_text: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObservationRef {
    pub observation_id: String,
    pub relation_kind: String,
    pub ordinal: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TurnMetadata {
    pub interaction_kind: String,
    pub finish_kind: String,
    pub user_initiated: Option<bool>,
    pub web_context: Option<serde_json::Value>,
}
impl Default for TurnMetadata {
    fn default() -> Self {
        Self {
            interaction_kind: "chat".into(),
            finish_kind: "complete".into(),
            user_initiated: Some(true),
            web_context: None,
        }
    }
}
impl TurnMetadata {
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(
            self.interaction_kind.as_str(),
            "chat" | "screen_question" | "observation_comment" | "proactive_care"
        ) || !matches!(
            self.finish_kind.as_str(),
            "complete" | "user_limit" | "provider_limit"
        ) {
            return Err("turn_metadata_invalid".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_or_sensitive_evidence_cannot_be_saved() {
        let raw = r#"{"usable":true,"sensitive":false,"confidence":0.9,"summary":"visible"}"#;
        assert!(VisualEvidence::parse(raw, false).is_ok());
        assert!(VisualEvidence::parse(raw, true).is_err());
        assert_eq!(
            VisualEvidence::parse(&raw.replace("0.9", "1.9"), false).unwrap_err(),
            "screen_vision_invalid"
        );
        assert_eq!(
            VisualEvidence::parse(
                &raw.replace("\"sensitive\":false", "\"sensitive\":true"),
                false
            )
            .unwrap_err(),
            "screen_sensitive"
        );
        assert!(VisualEvidence::parse(&raw.replace("visible", &"字".repeat(501)), false).is_err());
    }
}
