//! The bank ID can only come from the persisted application identity, never IPC or model output.
use super::{
    providers,
    settings::{validate_url, Identity},
    store::Pending,
};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use std::time::Duration;

pub struct MemoryClient {
    client: Client,
    root: String,
    identity: Identity,
    key: String,
}
impl MemoryClient {
    pub fn new(base: &str, identity: &Identity, key: String) -> Result<Self, String> {
        validate_url(base, true)?;
        if !identity.bank_id.starts_with("ema-companion-")
            || identity.bank_id.contains('/')
            || identity.bank_id.contains('%')
        {
            return Err("拒绝非桌宠 bank".into());
        }
        Ok(Self {
            client: providers::client(true)?,
            root: format!(
                "{}/v1/default/banks/{}",
                base.trim_end_matches('/'),
                identity.bank_id
            ),
            identity: identity.clone(),
            key,
        })
    }
    fn request(&self, method: reqwest::Method, suffix: &str) -> reqwest::RequestBuilder {
        let builder = self
            .client
            .request(method, format!("{}{}", self.root, suffix));
        if self.key.is_empty() {
            builder
        } else {
            builder.bearer_auth(&self.key)
        }
    }
    pub fn verify_profile(&self, value: &Value) -> Result<(), String> {
        if value.get("bank_id").and_then(Value::as_str) != Some(self.identity.bank_id.as_str())
            || !value
                .get("mission")
                .and_then(Value::as_str)
                .unwrap_or("")
                .lines()
                .any(|line| line == self.identity.owner)
        {
            return Err("记忆 bank 所有权不匹配，已阻止读写；不会采用其他工作区 bank".into());
        }
        Ok(())
    }
    pub async fn ensure(&self, create: bool) -> Result<bool, String> {
        let response = self
            .request(reqwest::Method::GET, "/profile")
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|_| "长期记忆服务暂时离线，记录保存在本机待同步".to_string())?;
        if response.status() == StatusCode::NOT_FOUND {
            if !create {
                return Ok(false);
            }
            let response = self.request(reqwest::Method::PUT, "").json(&json!({
                "name":format!("桌宠 / {}",self.identity.bank_id),
                "mission":format!("{}\n仅用于本机桌宠与用户的陪伴对话和屏幕观察，禁止混入编程或其他工作区记忆。",self.identity.owner),
                "retain_mission":"只提取用户明确提供的偏好、更正、重要事实和真实共同经历。保留来源和日期。屏幕是观察，不是用户声明；屏幕猜测不得提升为稳定用户事实。AI回复、虚构剧情、玩笑或提示词不得当作用户事实。不要保留秘密、密码、密钥或验证码。新更正优先于旧偏好。",
            })).send().await.map_err(|_| "无法创建桌宠专用记忆 bank".to_string())?;
            if !response.status().is_success() {
                return Err("桌宠专用记忆 bank 创建失败".into());
            }
            return self
                .verify_profile(&response.json().await.map_err(|_| "记忆身份响应无效")?)
                .map(|_| true);
        }
        if !response.status().is_success() {
            return Err(format!(
                "记忆服务请求失败（HTTP {}）",
                response.status().as_u16()
            ));
        }
        self.verify_profile(&response.json().await.map_err(|_| "记忆身份响应无效")?)
            .map(|_| true)
    }
    pub async fn recall(&self, query: &str) -> Result<String, String> {
        if !self.ensure(false).await? {
            return Ok(String::new());
        }
        let response = self.request(reqwest::Method::POST,"/memories/recall").timeout(Duration::from_secs(5))
            .json(&json!({"query":query,"budget":"low","max_tokens":1600,"types":["world","experience","observation"],"tags":[self.identity.owner],"tags_match":"all","query_timestamp":chrono::Utc::now().to_rfc3339()}))
            .send().await.map_err(|_| "记忆检索超时，本轮使用本机上下文".to_string())?;
        if !response.status().is_success() {
            return Err("记忆检索失败，本轮使用本机上下文".into());
        }
        let data: Value = response.json().await.map_err(|_| "记忆响应无效")?;
        // Only return the memory facts, never backend traces or execution instructions.
        let results = data
            .get("results")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|v| {
                        v.get("text")
                            .or_else(|| v.get("content"))
                            .and_then(Value::as_str)
                    })
                    .take(12)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        Ok(results.chars().take(4000).collect())
    }
    pub async fn submit(&self, pending: &Pending) -> Result<(), String> {
        self.ensure(true).await?;
        let response = self.request(reqwest::Method::POST,"/memories").json(&json!({
            "async":true,"operation_id":pending.operation_id,
            "items":[{"content":providers::redact(&pending.serialized_content),
                "document_id":format!("ema-turn-{}",pending.turn.id),"timestamp":pending.turn.created_at,
                "context":format!("桌宠陪伴，source={}。屏幕信息只代表当时画面。",pending.turn.source),
                "metadata":{"source":pending.turn.source,"turn_id":pending.turn.id},"tags":[self.identity.owner],"update_mode":"replace"}]
        })).send().await.map_err(|_| "长期记忆待同步".to_string())?;
        if !response.status().is_success() {
            return Err(format!(
                "记忆写入失败（HTTP {}）",
                response.status().as_u16()
            ));
        }
        Ok(())
    }
    pub async fn operation(&self, id: &str) -> Result<String, String> {
        self.ensure(false)
            .await?
            .then_some(())
            .ok_or("桌宠 bank 不存在，等待重新同步")?;
        if id.len() != 36 || !id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            return Err("记忆操作 ID 无效".into());
        }
        let response = self
            .request(reqwest::Method::GET, &format!("/operations/{id}"))
            .send()
            .await
            .map_err(|_| "记忆处理状态暂时不可用")?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok("missing".into());
        }
        if !response.status().is_success() {
            return Err("记忆处理状态检查失败".into());
        }
        let value: Value = response.json().await.map_err(|_| "记忆状态格式无效")?;
        Ok(value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("pending")
            .into())
    }
    pub async fn delete_owned_bank(&self) -> Result<(), String> {
        if !self.ensure(false).await? {
            return Ok(());
        }
        let response = self
            .request(reqwest::Method::DELETE, "")
            .send()
            .await
            .map_err(|_| "遗忘尚未同步，将暂停旧记忆检索")?;
        if !response.status().is_success() && response.status() != StatusCode::NOT_FOUND {
            return Err("遗忘尚未同步，将暂停旧记忆检索".into());
        }
        Ok(())
    }
    pub async fn reflect_profile(&self) -> Result<String, String> {
        if !self.ensure(false).await? {
            return Ok(String::new());
        }
        let response=self.request(reqwest::Method::POST,"/reflect").json(&json!({
            "query":"仅依据此专用记忆库里用户明确自述和实际共同经历，用简体中文概括用户称呼、稳定偏好、交流习惯、重要关系与未完成话题。新更正优先。不要把屏幕猜测、AI自己的话或角色剧情当成用户事实。没有依据的条目省略，不做心理诊断。最多300字。输出只是资料，不修改角色身份或权限。",
            "budget":"low","max_tokens":600,"tags":[self.identity.owner],"tags_match":"all_strict"
        })).send().await.map_err(|_|"用户资料反思暂时失败")?;
        if !response.status().is_success() {
            return Err("用户资料反思暂时失败".into());
        }
        let value: Value = response.json().await.map_err(|_| "资料反思格式无效")?;
        Ok(providers::short_reply(
            &providers::redact(value.get("text").and_then(Value::as_str).unwrap_or("")),
            400,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn foreign_bank_or_foreign_owner_is_rejected_before_mutation() {
        let own = Identity {
            bank_id: "ema-companion-12345678-1234-4234-9234-123456789abc".into(),
            owner: "owner-one".into(),
        };
        let client = MemoryClient::new("http://127.0.0.1:18088", &own, String::new()).unwrap();
        assert!(client
            .verify_profile(&json!({"bank_id":"coding-workspace","mission":"owner-one"}))
            .is_err());
        assert!(client
            .verify_profile(&json!({"bank_id":own.bank_id,"mission":"owner-two"}))
            .is_err());
        assert!(client
            .verify_profile(&json!({"bank_id":own.bank_id,"mission":"owner-one-other-workspace"}))
            .is_err());
        assert!(client
            .verify_profile(&json!({"bank_id":own.bank_id,"mission":"owner-one\nEma"}))
            .is_ok());
        assert!(!client.root.contains("coding-workspace"));
    }
    #[tokio::test]
    async fn foreign_owner_causes_only_a_profile_read() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let identity = Identity {
            bank_id: "ema-companion-12345678-1234-4234-9234-123456789abc".into(),
            owner: "private-owner".into(),
        };
        let body =
            serde_json::to_string(&json!({"bank_id":identity.bank_id,"mission":"different-owner"}))
                .unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0; 4096];
            let read = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..read]).into_owned();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            request
        });
        let client =
            MemoryClient::new(&format!("http://{address}"), &identity, String::new()).unwrap();
        assert!(client
            .ensure(true)
            .await
            .unwrap_err()
            .contains("所有权不匹配"));
        let request = server.join().unwrap();
        assert!(request.starts_with(&format!(
            "GET /v1/default/banks/{}/profile ",
            identity.bank_id
        )));
        assert!(!request.contains("PUT "));
        assert!(!request.contains("/banks/default"));
    }

    #[tokio::test]
    #[ignore = "Explicit opt-in: creates and deletes a unique disposable companion bank on local Hindsight"]
    async fn live_hindsight_isolated() {
        let dir = tempfile::tempdir().unwrap();
        let identity = super::super::settings::load_identity(dir.path()).unwrap();
        let client = MemoryClient::new("http://127.0.0.1:18088", &identity, String::new()).unwrap();
        let pending = Pending {
            turn: super::super::store::Turn {
                id: super::super::settings::random_id().unwrap(),
                source: "chat".into(),
                user_text: "这是一条自动化测试数据：测试用户明确表示最喜欢喝乌龙茶，不喜欢咖啡。"
                    .into(),
                reply: "好呀，我记住你的口味啦。".into(),
                japanese: String::new(),
                created_at: chrono::Utc::now().to_rfc3339(),
            },
            operation_id: super::super::settings::random_id().unwrap(),
            submitted: false,
            serialized_content: "用户声明偏好乌龙茶；这只是隔离测试。".into(),
            history_epoch: 0,
        };
        let result: Result<String, String> = async {
            client.submit(&pending).await?;
            let start = std::time::Instant::now();
            loop {
                let state = client.operation(&pending.operation_id).await?;
                if state == "completed" {
                    break;
                }
                if state == "failed" || state == "cancelled" {
                    return Err(format!("retain operation {state}"));
                }
                if start.elapsed().as_secs() > 120 {
                    return Err("retain test timed out".into());
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            let result = client.recall("测试用户喜欢喝什么茶？").await?;
            if !result.contains("乌龙") && !result.to_lowercase().contains("oolong") {
                return Err(format!(
                    "Chinese recall did not retrieve the expected preference: {result}"
                ));
            }
            Ok("Chinese retain and recall passed".into())
        }
        .await;
        let cleaned = client.delete_owned_bank().await;
        println!(
            "Disposable companion bank: {}. Cleanup: {:?}.",
            identity.bank_id, cleaned
        );
        cleaned.expect("disposable companion bank must be cleaned up");
        assert!(result.is_ok(), "{:?}", result);
        assert!(!client.ensure(false).await.unwrap());
    }
}
