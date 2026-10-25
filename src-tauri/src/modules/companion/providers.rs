use super::settings::{Endpoint, Secrets, Settings, ThinkingFormat, ThinkingLevel};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Clone, Debug, Serialize)]
pub struct ThinkingCapability {
    pub supported: bool,
    pub levels: Vec<ThinkingLevel>,
    pub message: String,
}

pub fn official_deepseek(endpoint: &Endpoint) -> bool {
    let Ok(url) = reqwest::Url::parse(&endpoint.base_url) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str() == Some("api.deepseek.com")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url.query().is_none()
        && url.fragment().is_none()
        && matches!(
            url.path().trim_end_matches('/'),
            "" | "/v1" | "/chat/completions" | "/v1/chat/completions"
        )
}

pub fn thinking_capability(endpoint: &Endpoint) -> ThinkingCapability {
    let supported = endpoint.thinking_format != ThinkingFormat::Auto
        || (official_deepseek(endpoint)
            && matches!(
                endpoint.model.as_str(),
                "deepseek-flash" | "deepseek-v4-pro"
            ));
    ThinkingCapability {
        supported,
        levels: if supported { vec![ThinkingLevel::Off, ThinkingLevel::Low, ThinkingLevel::High, ThinkingLevel::Max] } else { vec![ThinkingLevel::Off] },
        message: match endpoint.thinking_format {
            ThinkingFormat::Deepseek => "已显式选择 DeepSeek 兼容参数；仅在服务支持 thinking 与 low/high/max 时使用",
            ThinkingFormat::ReasoningEffort => "已显式选择 reasoning_effort 参数 none/low/high/max；各模型支持值不同，不能关闭思考的模型会拒绝 none",
            _ if supported => "支持关闭、低、高、最高思考强度",
            _ => "当前服务或模型尚未适配思考参数；不会发送不兼容字段，服务自身默认行为仍适用",
        }.into(),
    }
}

fn apply_thinking(body: &mut Value, endpoint: &Endpoint, configured: bool) -> Result<(), String> {
    let level = if configured {
        endpoint.thinking
    } else {
        ThinkingLevel::Off
    };
    if endpoint.thinking_format == ThinkingFormat::ReasoningEffort {
        body["reasoning_effort"] = match level {
            ThinkingLevel::Off => json!("none"),
            _ => json!(level),
        };
        body.as_object_mut().unwrap().remove("temperature");
        if let Some(tokens) = body.as_object_mut().unwrap().remove("max_tokens") {
            body["max_completion_tokens"] = tokens;
        }
    } else if thinking_capability(endpoint).supported {
        body["thinking"] =
            json!({"type": if level == ThinkingLevel::Off { "disabled" } else { "enabled" }});
        if level != ThinkingLevel::Off {
            body["reasoning_effort"] = json!(level);
            body.as_object_mut().unwrap().remove("temperature");
        }
    } else if level != ThinkingLevel::Off {
        return Err("当前模型尚未适配思考强度，请关闭该选项或使用受支持的官方模型".into());
    } else if official_deepseek(endpoint) {
        // The official API's documented off toggle also applies to legacy model aliases.
        body["thinking"] = json!({"type":"disabled"});
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct CompletionResult {
    pub text: String,
    pub finish_reason: Option<String>,
    pub usage: Option<Value>,
}

pub fn client(local: bool) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(45))
        .connect_timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none());
    if local {
        builder = builder.no_proxy();
    }
    builder.build().map_err(|_| "无法初始化 API 连接".into())
}
pub fn chat_url(base: &str) -> String {
    let value = base.trim_end_matches('/');
    if value.ends_with("/chat/completions") {
        value.into()
    } else {
        format!("{value}/chat/completions")
    }
}
fn local_endpoint(base: &str) -> bool {
    reqwest::Url::parse(base)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .map(|h| matches!(h.as_str(), "127.0.0.1" | "localhost" | "::1" | "[::1]"))
        .unwrap_or(false)
}
pub async fn completion(
    endpoint: &Endpoint,
    key: &str,
    messages: Vec<Value>,
    max_tokens: u32,
    json_output: bool,
) -> Result<String, String> {
    let result =
        completion_response(endpoint, key, messages, max_tokens, json_output, false).await?;
    if result
        .finish_reason
        .as_deref()
        .is_some_and(|reason| reason != "stop")
    {
        return Err("模型未完整生成内容，请重试".into());
    }
    Ok(result.text)
}

pub async fn completion_configured(
    endpoint: &Endpoint,
    key: &str,
    messages: Vec<Value>,
    max_tokens: u32,
    json_output: bool,
) -> Result<String, String> {
    let result =
        completion_response(endpoint, key, messages, max_tokens, json_output, true).await?;
    if result
        .finish_reason
        .as_deref()
        .is_some_and(|reason| reason != "stop")
    {
        return Err("模型未完整生成内容，请重试".into());
    }
    Ok(result.text)
}

pub async fn completion_response(
    endpoint: &Endpoint,
    key: &str,
    messages: Vec<Value>,
    max_tokens: u32,
    json_output: bool,
    configured: bool,
) -> Result<CompletionResult, String> {
    if endpoint.base_url.is_empty() || endpoint.model.is_empty() || key.is_empty() {
        return Err("请先配置模型地址、模型名称和 API 密钥".into());
    }
    super::settings::validate_url(&endpoint.base_url, false)?;
    let mut body = json!({"model":endpoint.model,"messages":messages,"max_tokens":max_tokens,"temperature":0.7,"stream":false});
    apply_thinking(&mut body, endpoint, configured)?;
    if json_output {
        body["response_format"] = json!({"type":"json_object"});
    }
    let resp = client(local_endpoint(&endpoint.base_url))?
        .post(chat_url(&endpoint.base_url))
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(|_| "模型连接超时或网络不可用，请检查设置".to_string())?;
    let code = resp.status();
    if !code.is_success() {
        return Err(response_error(resp).await);
    }
    let data: Value = resp
        .json()
        .await
        .map_err(|_| "模型返回格式无效".to_string())?;
    let text = data
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if text.is_empty() {
        return Err("模型没有返回可显示的内容".into());
    }
    Ok(CompletionResult {
        text: text.into(),
        finish_reason: data
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            .map(str::to_owned),
        usage: data.get("usage").filter(|value| value.is_object()).cloned(),
    })
}

/// SSE frames are decoded only after a complete line arrives, so a transport
/// chunk may split a UTF-8 character, CRLF, JSON value, or event delimiter.
#[derive(Default)]
struct EventDecoder {
    pending: Vec<u8>,
    data: Vec<String>,
}
impl EventDecoder {
    fn line(&mut self, bytes: &[u8], output: &mut Vec<String>) -> Result<(), String> {
        let line = std::str::from_utf8(bytes)
            .map_err(|_| "模型流式返回的文字编码无效".to_string())?
            .trim_end_matches('\r');
        if line.is_empty() {
            if !self.data.is_empty() {
                output.push(self.data.join("\n"));
                self.data.clear();
            }
        } else if let Some(value) = line.strip_prefix("data:") {
            self.data
                .push(value.strip_prefix(' ').unwrap_or(value).into());
        }
        Ok(())
    }
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>, String> {
        self.pending.extend_from_slice(bytes);
        let mut output = Vec::new();
        while let Some(index) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=index).collect();
            self.line(&line[..line.len() - 1], &mut output)?;
        }
        Ok(output)
    }
    fn finish(&mut self) -> Result<Vec<String>, String> {
        let mut output = Vec::new();
        let last = std::mem::take(&mut self.pending);
        if !last.is_empty() {
            self.line(&last, &mut output)?;
        }
        self.line(&[], &mut output)?;
        Ok(output)
    }
}

fn response_text(data: &Value, stream: bool) -> Result<&str, String> {
    if data.get("error").is_some() {
        return Err("模型返回错误，请检查模型配置或服务额度".into());
    }
    Ok(if stream {
        data.pointer("/choices/0/delta/content")
            .or_else(|| data.pointer("/choices/0/message/content"))
            .or_else(|| data.pointer("/choices/0/text"))
    } else {
        data.pointer("/choices/0/message/content")
    }
    .and_then(Value::as_str)
    .unwrap_or(""))
}

fn append_event(
    event: &str,
    text: &mut String,
    on_text: &mut impl FnMut(&str) -> Result<bool, String>,
) -> Result<bool, String> {
    if event.trim() == "[DONE]" {
        return Ok(false);
    }
    let data: Value =
        serde_json::from_str(event).map_err(|_| "模型流式返回格式无效".to_string())?;
    let delta = response_text(&data, true)?;
    if !delta.is_empty() {
        text.push_str(delta);
        return on_text(text);
    }
    Ok(true)
}

/// Invoke the model once and publish cumulative text as it arrives. The callback
/// may stop reading once the user's chosen character limit has been reached.
/// No application token limit is imposed; model/provider limits still apply.
pub async fn streaming_completion(
    endpoint: &Endpoint,
    key: &str,
    messages: Vec<Value>,
    on_text: impl FnMut(&str) -> Result<bool, String>,
) -> Result<String, String> {
    Ok(
        streaming_impl(endpoint, key, messages, None, false, on_text)
            .await?
            .text,
    )
}

pub async fn streaming_completion_configured(
    endpoint: &Endpoint,
    key: &str,
    messages: Vec<Value>,
    on_text: impl FnMut(&str) -> Result<bool, String>,
) -> Result<String, String> {
    Ok(streaming_impl(endpoint, key, messages, None, true, on_text)
        .await?
        .text)
}

pub async fn streaming_response(
    endpoint: &Endpoint,
    key: &str,
    messages: Vec<Value>,
    max_tokens: u32,
    on_text: impl FnMut(&str) -> Result<bool, String>,
) -> Result<CompletionResult, String> {
    streaming_impl(endpoint, key, messages, Some(max_tokens), true, on_text).await
}

async fn streaming_impl(
    endpoint: &Endpoint,
    key: &str,
    messages: Vec<Value>,
    max_tokens: Option<u32>,
    configured: bool,
    mut on_text: impl FnMut(&str) -> Result<bool, String>,
) -> Result<CompletionResult, String> {
    if endpoint.base_url.is_empty() || endpoint.model.is_empty() || key.is_empty() {
        return Err("请先配置模型地址、模型名称和 API 密钥".into());
    }
    super::settings::validate_url(&endpoint.base_url, false)?;
    let mut body =
        json!({"model":endpoint.model,"messages":messages,"temperature":0.7,"stream":true});
    if let Some(tokens) = max_tokens {
        body["max_tokens"] = json!(tokens);
    }
    apply_thinking(&mut body, endpoint, configured)?;
    // Long replies may take minutes. Timeout each network read rather than the
    // entire reply; dropping this future still cancels the in-flight request.
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .read_timeout(Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::none());
    if local_endpoint(&endpoint.base_url) {
        builder = builder.no_proxy();
    }
    let mut response = builder
        .build()
        .map_err(|_| "无法初始化 API 连接".to_string())?
        .post(chat_url(&endpoint.base_url))
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(|_| "模型连接超时或网络不可用，请检查设置".to_string())?;
    if !response.status().is_success() {
        return Err(response_error(response).await);
    }
    let mut decoder = EventDecoder::default();
    let mut text = String::new();
    let mut fallback = Vec::new();
    let mut saw_event = false;
    let mut finish_reason = None;
    let mut usage = None;
    while let Some(bytes) = response
        .chunk()
        .await
        .map_err(|_| "模型流式连接中断，请重试".to_string())?
    {
        if !saw_event {
            fallback.extend_from_slice(&bytes);
        }
        for event in decoder.push(&bytes)? {
            saw_event = true;
            fallback.clear();
            capture_finish_reason(&event, &mut finish_reason);
            capture_usage(&event, &mut usage);
            if !append_event(&event, &mut text, &mut on_text)? {
                return Ok(CompletionResult {
                    text: nonempty_reply(text)?,
                    finish_reason,
                    usage,
                });
            }
        }
    }
    // Some compatible providers omit the final blank line or [DONE] marker.
    for event in decoder.finish()? {
        saw_event = true;
        capture_finish_reason(&event, &mut finish_reason);
        capture_usage(&event, &mut usage);
        if !append_event(&event, &mut text, &mut on_text)? {
            return Ok(CompletionResult {
                text: nonempty_reply(text)?,
                finish_reason,
                usage,
            });
        }
    }
    if !saw_event {
        // Compatible services may ignore stream=true and return one JSON body.
        // Accept that response without issuing a second request.
        let data: Value =
            serde_json::from_slice(&fallback).map_err(|_| "模型返回格式无效".to_string())?;
        text = response_text(&data, false)?.into();
        finish_reason = data
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            .map(str::to_owned);
        usage = data.get("usage").filter(|value| value.is_object()).cloned();
        if !text.is_empty() {
            on_text(&text)?;
        }
    }
    Ok(CompletionResult {
        text: nonempty_reply(text)?,
        finish_reason,
        usage,
    })
}

fn context_limit_error(status: u16, data: &Value) -> bool {
    matches!(status, 400 | 413)
        && [data.pointer("/error/code"), data.pointer("/error/type")]
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .any(|v| matches!(v, "context_length_exceeded" | "max_context_length_exceeded"))
}
async fn response_error(mut response: reqwest::Response) -> String {
    let status = response.status().as_u16();
    let generic = format!("模型请求失败（HTTP {status}），请检查密钥、模型或额度");
    if !matches!(status, 400 | 413) {
        return generic;
    }
    let parsed = tokio::time::timeout(Duration::from_secs(2), async {
        let mut body = Vec::new();
        while let Ok(Some(chunk)) = response.chunk().await {
            if body.len() + chunk.len() > 8192 {
                return None;
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice::<Value>(&body).ok()
    })
    .await
    .ok()
    .flatten();
    if parsed
        .as_ref()
        .is_some_and(|data| context_limit_error(status, data))
    {
        "context_length_exceeded".into()
    } else {
        generic
    }
}

fn capture_usage(event: &str, usage: &mut Option<Value>) {
    if let Ok(data) = serde_json::from_str::<Value>(event) {
        if let Some(value) = data.get("usage").filter(|value| value.is_object()) {
            *usage = Some(value.clone());
        }
    }
}

fn capture_finish_reason(event: &str, reason: &mut Option<String>) {
    if let Ok(data) = serde_json::from_str::<Value>(event) {
        if let Some(value) = data
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
        {
            *reason = Some(value.into());
        }
    }
}

fn nonempty_reply(text: String) -> Result<String, String> {
    if text.trim().is_empty() {
        Err("模型没有返回可显示的内容".into())
    } else {
        Ok(text)
    }
}

/// Keep the cumulative display stable while withholding a possible unfinished
/// credential. Otherwise a secret split across deltas could flash before the
/// complete value is long enough for the normal redactor to recognize it.
pub fn display_reply(text: &str, max_chars: usize, done: bool) -> String {
    let safe = if done { text } else { safe_stream_prefix(text) };
    let clean = super::reply_text::without_internal_labels(safe, done);
    let clean = redact(&clean).replace("```", "");
    let clean = clean.trim();
    if max_chars == 0 {
        clean.into()
    } else {
        clean.chars().take(max_chars).collect()
    }
}

fn safe_stream_prefix(text: &str) -> &str {
    let text = text.trim_end_matches('`');
    static POSSIBLE_SECRET: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = POSSIBLE_SECRET.get_or_init(|| {
        regex::Regex::new(r"(?i)(?:sk-[a-z0-9_-]*|bearer\s+[a-z0-9._-]*|(?:api[_ -]?key|password|密码|验证码)\s*[:：=]?\s*[^\s，。;；]*)$").unwrap()
    });
    if let Some(secret) = pattern.find(text) {
        return &text[..secret.start()];
    }
    // Withhold incomplete markers, including when the marker itself is split.
    for marker in [
        "sk-",
        "bearer ",
        "api_key",
        "api-key",
        "api key",
        "apikey",
        "password",
        "密码",
        "验证码",
    ] {
        for (end, _) in marker
            .char_indices()
            .skip(1)
            .chain(std::iter::once((marker.len(), ' ')))
        {
            let prefix = &marker[..end];
            if text.len() >= prefix.len() {
                let start = text.len() - prefix.len();
                if text.is_char_boundary(start) && text[start..].eq_ignore_ascii_case(prefix) {
                    return &text[..start];
                }
            }
        }
    }
    text
}
pub fn short_reply(text: &str, max: usize) -> String {
    let clean = text.trim().trim_matches('"').replace("```", "");
    if clean.chars().count() <= max {
        return clean;
    }
    let prefix: String = clean.chars().take(max.saturating_sub(1)).collect();
    if let Some((idx, ch)) = prefix
        .char_indices()
        .rev()
        .find(|(i, c)| *i > prefix.len() / 3 && "。！？!?\n".contains(*c))
    {
        prefix[..idx + ch.len_utf8()].trim().into()
    } else {
        format!("{}…", prefix.trim_end())
    }
}
pub fn redact(text: &str) -> String {
    static KEY: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let key = KEY.get_or_init(|| regex::Regex::new(r"(?i)(?:sk-[a-z0-9_-]{12,}|bearer\s+[a-z0-9._-]{12,}|(?:api[_ -]?key|password|密码|验证码)\s*[:：=]\s*[^\s，。;；]{4,})").unwrap());
    key.replace_all(text, "[敏感内容已隐藏]").into_owned()
}
/// Persona is reference data, not a competing instruction for output language.
pub async fn translate_for_role(
    settings: &Settings,
    secrets: &Secrets,
    text: &str,
    role: &super::character::RuntimeProfile,
) -> Result<String, String> {
    translate_role_request(&settings.chat, &secrets.get("chat")?, text, role).await
}
async fn translate_role_request(
    endpoint: &Endpoint,
    key: &str,
    text: &str,
    role: &super::character::RuntimeProfile,
) -> Result<String, String> {
    let prompt = role.translation_prompt();
    for attempt in 0..2 {
        let instruction = if attempt == 0 {
            prompt.clone()
        } else {
            format!("{prompt}\n上一轮译文未通过朗读语言检查。重新翻译全文，日语须使用自然的日语句子及假名，英文须使用英文，不沿用中文原文。")
        };
        let result = streaming_impl(endpoint, key, vec![
            json!({"role":"system","content":instruction}),
            json!({"role":"user","content":json!({"character_style_reference":role.persona,"text_to_translate":text}).to_string()}),
        ], None, false, |_| Ok(true)).await?;
        if result.finish_reason.as_deref().is_some_and(|v| v != "stop") {
            return Err("朗读译文未完整生成".into());
        }
        let translated = redact(result.text.trim());
        if translation_matches_locale(&translated, &role.voice_locale) {
            return Ok(translated);
        }
    }
    Err("朗读译文未符合所选语言，本次仅显示文字".into())
}
fn translation_matches_locale(text: &str, locale: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let kana = text.chars().any(|c| matches!(c, '\u{3041}'..='\u{3096}' | '\u{30a1}'..='\u{30fa}' | '\u{ff66}'..='\u{ff9d}'));
    let han = text
        .chars()
        .any(|c| matches!(c, '\u{3400}'..='\u{9fff}' | '\u{20000}'..='\u{3134f}'));
    match locale {
        // Kanji-only words can belong to either language. Ask for a natural
        // sentence rather than silently feeding an unverified Chinese echo to TTS.
        "ja" | "jp" => kana || (!han && !text.chars().any(char::is_lowercase)),
        "en" => !han && !kana,
        _ => true,
    }
}
pub async fn synthesize(
    settings: &Settings,
    secrets: &Secrets,
    text: &str,
) -> Result<Vec<u8>, String> {
    if settings.voice_id.trim().is_empty() {
        return Err("请先配置当前角色的私有声音 ID".into());
    }
    let key = secrets.get("speech")?;
    if key.is_empty() {
        return Err("请先配置 Fish Audio 密钥".into());
    }
    let resp = client(false)?.post(format!("{}/v1/tts", settings.speech.base_url.trim_end_matches('/')))
        .bearer_auth(key).header("model", &settings.speech.model)
        .json(&json!({"text":text,"reference_id":settings.voice_id,"format":"mp3","prosody":{"speed":settings.speech_speed,"volume":0,"normalize_loudness":true}}))
        .send().await.map_err(|_| "语音服务暂时无法连接".to_string())?;
    if !resp.status().is_success() {
        return Err(format!("语音合成失败（HTTP {}）", resp.status().as_u16()));
    }
    if resp.content_length().unwrap_or(0) > 12_000_000 {
        return Err("语音数据过大".into());
    }
    let data = resp.bytes().await.map_err(|_| "语音下载失败".to_string())?;
    if data.len() > 12_000_000 || data.is_empty() {
        return Err("语音返回无效".into());
    }
    Ok(data.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_errors_are_recognized_by_codes_instead_of_generic_http_failures() {
        assert!(context_limit_error(
            400,
            &json!({"error":{"code":"context_length_exceeded"}})
        ));
        assert!(!context_limit_error(
            401,
            &json!({"error":{"code":"context_length_exceeded"}})
        ));
        assert!(!context_limit_error(
            400,
            &json!({"error":{"code":"invalid_api_key","message":"context_length_exceeded"}})
        ));
    }
    use std::io::{Read, Write};

    fn japanese_role() -> super::super::character::RuntimeProfile {
        super::super::character::RuntimeProfile {
            id: "test".into(),
            display_name: "伙伴".into(),
            persona: "自然地用简体中文回复。语气友善，日语自称ボク。".into(),
            voice_locale: "ja".into(),
            voice_id: "fake-voice".into(),
        }
    }
    #[tokio::test]
    async fn japanese_translation_keeps_chinese_persona_in_reference_data() {
        let (endpoint, server) = mock_response(r#"{"choices":[{"message":{"content":"こんにちは。今日は一緒にお話ししましょう。"},"finish_reason":"stop"}]}"#.into(), "application/json");
        let text = translate_role_request(
            &endpoint,
            "fake",
            "你好，今天一起聊聊吧。",
            &japanese_role(),
        )
        .await
        .unwrap();
        assert!(text.contains("こんにちは"));
        let request = server.join().unwrap();
        let system = request["messages"][0]["content"].as_str().unwrap();
        assert!(system.contains("目标朗读语言：日语"));
        assert!(!system.contains("自然地用简体中文回复"));
        let reference: Value =
            serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(reference["text_to_translate"], "你好，今天一起聊聊吧。");
        assert!(reference["character_style_reference"]
            .as_str()
            .unwrap()
            .contains("简体中文"));
    }
    #[tokio::test]
    async fn translation_retries_a_chinese_echo_before_any_tts_can_receive_it() {
        let (endpoint, server) = mock_responses(vec![
            r#"{"choices":[{"message":{"content":"你好，今天一起聊聊吧。"},"finish_reason":"stop"}]}"#.into(),
            r#"{"choices":[{"message":{"content":"こんにちは。今日は一緒にお話ししましょう。"},"finish_reason":"stop"}]}"#.into(),
        ], "application/json");
        let text = translate_role_request(
            &endpoint,
            "fake",
            "你好，今天一起聊聊吧。",
            &japanese_role(),
        )
        .await
        .unwrap();
        assert!(text.contains("こんにちは"));
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1]["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("未通过朗读语言检查"));
    }
    #[tokio::test]
    async fn wrong_language_or_truncated_translation_never_returns_speakable_text() {
        let bad =
            r#"{"choices":[{"message":{"content":"这仍然是中文回复。"},"finish_reason":"stop"}]}"#
                .to_string();
        let (endpoint, server) = mock_responses(vec![bad.clone(), bad], "application/json");
        assert!(
            translate_role_request(&endpoint, "fake", "这是中文。", &japanese_role())
                .await
                .unwrap_err()
                .contains("所选语言")
        );
        assert_eq!(server.join().unwrap().len(), 2);
        let (endpoint, server) = mock_response(
            r#"{"choices":[{"message":{"content":"こんにちは。"},"finish_reason":"length"}]}"#
                .into(),
            "application/json",
        );
        assert!(
            translate_role_request(&endpoint, "fake", "这是中文。", &japanese_role())
                .await
                .unwrap_err()
                .contains("未完整")
        );
        server.join().unwrap();
    }
    #[test]
    fn language_guard_rejects_chinese_echoes_without_rejecting_numbers_or_japanese() {
        assert!(!translation_matches_locale("", "ja"));
        assert!(!translation_matches_locale("这是中文回复。", "ja"));
        assert!(translation_matches_locale("日本語で話しましょう。", "ja"));
        assert!(translation_matches_locale("ｺﾝﾆﾁﾊ", "jp"));
        assert!(translation_matches_locale("2026/10/23", "ja"));
        assert!(translation_matches_locale("OK", "ja"));
        assert!(!translation_matches_locale("Hello there", "ja"));
        assert!(translation_matches_locale("Hello there", "en"));
        assert!(!translation_matches_locale("中文回复", "en"));
    }

    #[test]
    fn thinking_adapters_keep_configured_calls_separate_from_internal_tasks() {
        let mut endpoint = Settings::default().chat;
        endpoint.thinking = ThinkingLevel::Max;
        let mut body = json!({"temperature":0.7});
        apply_thinking(&mut body, &endpoint, true).unwrap();
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["reasoning_effort"], "max");
        assert!(body.get("temperature").is_none());
        let mut internal = json!({});
        apply_thinking(&mut internal, &endpoint, false).unwrap();
        assert_eq!(internal["thinking"]["type"], "disabled");
        assert!(internal.get("reasoning_effort").is_none());
        endpoint.base_url = "https://proxy.example/v1".into();
        assert!(!thinking_capability(&endpoint).supported);
        assert!(apply_thinking(&mut json!({}), &endpoint, true).is_err());
        endpoint.thinking = ThinkingLevel::Off;
        let mut unknown = json!({});
        apply_thinking(&mut unknown, &endpoint, true).unwrap();
        assert_eq!(unknown, json!({}));
        endpoint.thinking_format = ThinkingFormat::Deepseek;
        apply_thinking(&mut unknown, &endpoint, true).unwrap();
        assert_eq!(unknown["thinking"]["type"], "disabled");
        endpoint.thinking_format = ThinkingFormat::ReasoningEffort;
        let mut reasoning = json!({"max_tokens":8192,"temperature":0.7});
        apply_thinking(&mut reasoning, &endpoint, true).unwrap();
        assert_eq!(reasoning["reasoning_effort"], "none");
        assert_eq!(reasoning["max_completion_tokens"], 8192);
        assert!(reasoning.get("max_tokens").is_none());
    }
    #[test]
    fn official_origin_check_rejects_lookalikes_credentials_ports_and_paths() {
        for value in [
            "https://api.deepseek.com",
            "https://api.deepseek.com/v1/",
            "https://api.deepseek.com/v1/chat/completions",
        ] {
            assert!(official_deepseek(&Endpoint {
                base_url: value.into(),
                ..Default::default()
            }));
        }
        for value in [
            "http://api.deepseek.com",
            "https://api.deepseek.com.evil.example",
            "https://api.deepseek.com:8443",
            "https://user@api.deepseek.com",
            "https://api.deepseek.com/v2",
            "https://api.deepseek.com/?key=x",
            "https://api.deepseek.com/#a",
        ] {
            assert!(
                !official_deepseek(&Endpoint {
                    base_url: value.into(),
                    ..Default::default()
                }),
                "{value}"
            );
        }
    }

    #[tokio::test]
    async fn configured_stream_reports_budget_finish_reason_and_never_displays_reasoning() {
        let (mut endpoint, server) = mock_response(concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"private-reasoning\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"visible answer\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}],\"usage\":{\"completion_tokens\":17}}\n\n",
            "data: [DONE]\n\n"
        ).into(), "text/event-stream");
        endpoint.thinking_format = ThinkingFormat::Deepseek;
        endpoint.thinking = ThinkingLevel::Low;
        let mut seen = Vec::new();
        let result = streaming_response(&endpoint, "fake", vec![], 17, |text| {
            seen.push(text.to_string());
            Ok(true)
        })
        .await
        .unwrap();
        assert_eq!(result.text, "visible answer");
        assert_eq!(result.finish_reason.as_deref(), Some("length"));
        assert_eq!(result.usage.unwrap()["completion_tokens"], 17);
        assert_eq!(seen, ["visible answer"]);
        let request = server.join().unwrap();
        assert_eq!(request["max_tokens"], 17);
        assert_eq!(request["thinking"]["type"], "enabled");
        assert_eq!(request["reasoning_effort"], "low");
    }
    #[tokio::test]
    async fn nonstream_internal_tasks_disable_thinking_and_reject_truncated_summaries() {
        let (mut endpoint, server) = mock_response(
            r#"{"choices":[{"message":{"content":"cut summary"},"finish_reason":"length"}]}"#
                .into(),
            "application/json",
        );
        endpoint.thinking_format = ThinkingFormat::Deepseek;
        endpoint.thinking = ThinkingLevel::Max;
        assert!(completion(&endpoint, "fake", vec![], 50, false)
            .await
            .is_err());
        let request = server.join().unwrap();
        assert_eq!(request["thinking"]["type"], "disabled");
        assert_eq!(request["max_tokens"], 50);
    }

    #[test]
    fn sse_decodes_every_transport_boundary_and_keeps_the_final_tail() {
        let source = concat!(
            ": heartbeat\r\n\r\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"你好🙂\"}}]}\r\n\r\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"，完整的结尾。\"}}]}"
        );
        for split in 0..=source.len() {
            let mut decoder = EventDecoder::default();
            let mut events = decoder.push(&source.as_bytes()[..split]).unwrap();
            events.extend(decoder.push(&source.as_bytes()[split..]).unwrap());
            events.extend(decoder.finish().unwrap());
            let mut actual = String::new();
            for event in events {
                assert!(append_event(&event, &mut actual, &mut |_| Ok(true)).unwrap());
            }
            assert_eq!(actual, "你好🙂，完整的结尾。", "boundary {split}");
        }
        let mut decoder = EventDecoder::default();
        let mut events = Vec::new();
        for byte in source.as_bytes() {
            events.extend(decoder.push(&[*byte]).unwrap());
        }
        events.extend(decoder.finish().unwrap());
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn sse_ignores_metadata_and_stops_at_done_or_user_cap() {
        let mut text = String::new();
        let mut seen = Vec::new();
        let mut callback = |value: &str| {
            seen.push(value.to_string());
            Ok(value.chars().count() < 2)
        };
        assert!(append_event(
            r#"{"choices":[{"delta":{"role":"assistant"}}]}"#,
            &mut text,
            &mut callback
        )
        .unwrap());
        assert!(append_event(
            r#"{"choices":[{"delta":{"content":"你"}}]}"#,
            &mut text,
            &mut callback
        )
        .unwrap());
        assert!(!append_event(
            r#"{"choices":[{"delta":{"content":"好"}}]}"#,
            &mut text,
            &mut callback
        )
        .unwrap());
        assert_eq!(text, "你好");
        assert_eq!(seen, ["你", "你好"]);
        assert!(!append_event("[DONE]", &mut text, &mut |_| panic!(
            "DONE must not repeat content"
        ))
        .unwrap());
        assert!(append_event(
            r#"{"error":{"message":"provider error"}}"#,
            &mut text,
            &mut |_| Ok(true)
        )
        .is_err());
    }

    #[test]
    fn display_length_is_optional_and_unicode_safe_without_early_sentence_loss() {
        let text = "开头。还有很长的内容🙂".repeat(80);
        assert_eq!(display_reply(&text, 0, true), text);
        assert_eq!(
            display_reply(&text, 41, true),
            text.chars().take(41).collect::<String>()
        );
        assert_eq!(display_reply("你好🙂完整", 3, true), "你好🙂");
    }
    #[test]
    fn internal_id_echo_is_removed_before_display_storage_and_speech_length_limits() {
        let text="[实际发言 ID=f5ad2564-42a8-4858-befe-56d3060ce1e4]\n查询资料显示，尚无官方公布的日期。";
        let mut previous = String::new();
        for (end, _) in text
            .char_indices()
            .chain(std::iter::once((text.len(), ' ')))
        {
            let visible = display_reply(&text[..end], 0, false);
            assert!(visible.starts_with(&previous));
            assert!(!visible.contains("实际发言"));
            assert!(!visible.contains("f5ad2564"));
            previous = visible;
        }
        assert_eq!(display_reply(text, 6, true), "查询资料显示");
        assert_eq!(
            display_reply(text, 0, true),
            "查询资料显示，尚无官方公布的日期。"
        );
    }

    #[test]
    fn partial_credentials_and_fences_never_flash_during_streaming() {
        let text = "你好，密钥是 sk-abcdefghijklmnopqrstuvwx。结束。";
        for (index, _) in text
            .char_indices()
            .chain(std::iter::once((text.len(), ' ')))
        {
            let visible = display_reply(&text[..index], 0, false);
            assert!(!visible.contains("sk-"), "{visible}");
            assert!(!visible.contains("abcdef"), "{visible}");
        }
        assert!(display_reply(text, 0, true).contains("[敏感内容已隐藏]"));
        assert_eq!(display_reply("正文``", 0, false), "正文");
        assert_eq!(display_reply("正文```后文", 0, false), "正文后文");
        assert_eq!(display_reply("文本 密码：abc", 0, false), "文本");
    }

    fn mock_response(
        body: String,
        content_type: &str,
    ) -> (Endpoint, std::thread::JoinHandle<Value>) {
        let (endpoint, server) = mock_responses(vec![body], content_type);
        (
            endpoint,
            std::thread::spawn(move || server.join().unwrap().pop().unwrap()),
        )
    }
    fn mock_responses(
        bodies: Vec<String>,
        content_type: &str,
    ) -> (Endpoint, std::thread::JoinHandle<Vec<Value>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let content_type = content_type.to_string();
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for body in bodies {
                let header = format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let header_end = loop {
                    let mut buffer = [0u8; 4096];
                    let count = socket.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                    if let Some(index) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                        break index + 4;
                    }
                };
                let length = String::from_utf8_lossy(&request[..header_end])
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                while request.len() < header_end + length {
                    let mut buffer = [0u8; 4096];
                    let count = socket.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                }
                let value =
                    serde_json::from_slice(&request[header_end..header_end + length]).unwrap();
                socket.write_all(header.as_bytes()).unwrap();
                socket.write_all(body.as_bytes()).unwrap();
                requests.push(value);
            }
            requests
        });
        (
            Endpoint {
                base_url: format!("http://{address}/v1"),
                model: "mock".into(),
                ..Default::default()
            },
            handle,
        )
    }

    #[tokio::test]
    async fn streaming_request_has_no_fixed_token_budget_and_delivers_tail() {
        let (endpoint, server) = mock_response(
            concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"你好🙂\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"。结尾\"}}]}"
            )
            .into(),
            "text/event-stream",
        );
        let mut seen = Vec::new();
        let text = streaming_completion(&endpoint, "mock-key", vec![], |value| {
            seen.push(value.to_string());
            Ok(true)
        })
        .await
        .unwrap();
        assert_eq!(text, "你好🙂。结尾");
        assert_eq!(seen, ["你好🙂", "你好🙂。结尾"]);
        let body = server.join().unwrap();
        assert_eq!(body["stream"], true);
        assert!(body.get("max_tokens").is_none());
    }

    #[tokio::test]
    async fn provider_ignoring_stream_returns_json_once_without_retry() {
        let (endpoint, server) = mock_response(
            r#"{"choices":[{"message":{"content":"兼容完整回复🙂"}}]}"#.into(),
            "application/json",
        );
        let mut seen = Vec::new();
        let text = streaming_completion(&endpoint, "mock-key", vec![], |value| {
            seen.push(value.to_string());
            Ok(true)
        })
        .await
        .unwrap();
        assert_eq!(text, "兼容完整回复🙂");
        assert_eq!(seen, [text]);
        server.join().unwrap();
    }
    #[test]
    fn reply_cap_preserves_unicode_and_prefers_sentences() {
        let text = "今天辛苦啦。".repeat(60);
        let reply = short_reply(&text, 120);
        assert!(reply.chars().count() <= 120);
        assert!(reply.ends_with('。'));
        assert_eq!(short_reply("你好。", 120), "你好。");
    }
    #[test]
    fn addresses_and_secrets() {
        assert_eq!(
            chat_url("https://api.deepseek.com/"),
            "https://api.deepseek.com/chat/completions"
        );
        assert_eq!(
            chat_url("https://server/v1/chat/completions"),
            "https://server/v1/chat/completions"
        );
        assert!(!redact("api_key: sk-abcdefghijklmnopqrstuvwx").contains("abcdefghijkl"));
    }
}
