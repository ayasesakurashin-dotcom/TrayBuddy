use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Endpoint {
    pub base_url: String,
    pub model: String,
    #[serde(skip_serializing)]
    pub api_key: String,
    pub has_key: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub schema_version: u32,
    pub input_enabled: bool,
    pub input_collapsed: bool,
    pub speech_enabled: bool,
    pub observation_enabled: bool,
    pub memory_enabled: bool,
    pub chat: Endpoint,
    pub vision: Endpoint,
    pub speech: Endpoint,
    pub memory: Endpoint,
    pub voice_id: String,
    pub speech_speed: f32,
    pub observation_interval_secs: u64,
    pub capture_scope: String,
    pub capture_region: [i32; 4],
    pub excluded_windows: Vec<String>,
    pub reply_max_chars: usize,
    pub nickname: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            input_enabled: true,
            input_collapsed: false,
            speech_enabled: true,
            observation_enabled: false,
            memory_enabled: true,
            chat: Endpoint {
                base_url: "https://api.deepseek.com".into(),
                model: "deepseek-flash".into(),
                ..Default::default()
            },
            vision: Endpoint::default(),
            speech: Endpoint {
                base_url: "https://api.fish.audio".into(),
                model: "s2.1-pro-free".into(),
                ..Default::default()
            },
            memory: Endpoint {
                base_url: "http://127.0.0.1:18088".into(),
                ..Default::default()
            },
            voice_id: String::new(),
            speech_speed: 1.0,
            observation_interval_secs: 300,
            capture_scope: "foreground".into(),
            capture_region: [0, 0, 800, 600],
            excluded_windows: vec![],
            // Zero leaves reply length to the user and the selected model.
            reply_max_chars: 0,
            nickname: String::new(),
        }
    }
}

pub fn validate_url(value: &str, local_only: bool) -> Result<(), String> {
    let url = reqwest::Url::parse(value).map_err(|_| "API 地址无效".to_string())?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("API 地址不能含凭据、查询参数或锚点".into());
    }
    let local = matches!(
        url.host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
    );
    if local_only && (!local || url.path().trim_matches('/') != "") {
        return Err("记忆地址仅允许本机服务根地址，不能填写 bank 或工作区路径".into());
    }
    if url.scheme() != "https" && !(url.scheme() == "http" && local) {
        return Err("外部 API 必须使用 HTTPS；本机服务可使用 HTTP".into());
    }
    Ok(())
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("不支持的 AI 设置版本，原文件已保留".into());
        }
        for endpoint in [&self.chat, &self.vision, &self.speech] {
            if !endpoint.base_url.is_empty() {
                validate_url(&endpoint.base_url, false)?;
            }
        }
        validate_url(&self.memory.base_url, true)?;
        if !(30..=86400).contains(&self.observation_interval_secs) {
            return Err("观察周期须在 30 秒至 24 小时之间".into());
        }
        if !matches!(
            self.capture_scope.as_str(),
            "foreground" | "screen" | "region"
        ) {
            return Err("截图范围无效".into());
        }
        if self.capture_region[2] <= 0
            || self.capture_region[3] <= 0
            || self.capture_region[2] > 16384
            || self.capture_region[3] > 16384
        {
            return Err("截图区域尺寸无效".into());
        }
        if !(0.6..=1.5).contains(&self.speech_speed) {
            return Err("语速须在 0.6—1.5 之间".into());
        }
        if self.nickname.chars().count() > 40 || self.excluded_windows.len() > 100 {
            return Err("设置内容过长".into());
        }
        Ok(())
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    let mut file = fs::File::create(&tmp).map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    if path.exists() {
        fs::copy(path, path.with_extension("bak")).map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn load_settings(dir: &Path) -> Result<Settings, String> {
    let path = dir.join("settings.json");
    if !path.exists() {
        return Ok(Settings::default());
    }
    let value: Settings = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|_| "AI 设置损坏，已停止加载并保留原文件；请从备份恢复".to_string())?;
    value.validate()?;
    Ok(value)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub bank_id: String,
    pub owner: String,
}

pub fn random_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|e| e.to_string())?;
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    Ok(format!("{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}", bytes[0],bytes[1],bytes[2],bytes[3],bytes[4],bytes[5],bytes[6],bytes[7],bytes[8],bytes[9],bytes[10],bytes[11],bytes[12],bytes[13],bytes[14],bytes[15]))
}

pub fn load_identity(dir: &Path) -> Result<Identity, String> {
    let path = dir.join("identity.json");
    let identity: Identity = if path.exists() {
        serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
            .map_err(|_| "桌宠记忆身份文件损坏，禁止自动创建或使用其他 bank".to_string())?
    } else {
        if dir.join("history.sqlite").exists() {
            return Err("桌宠记忆身份文件丢失，已停止接入以防 bank 混淆".into());
        }
        let value = Identity {
            bank_id: format!("ema-companion-{}", random_id()?),
            owner: format!("traybuddy-ema-owner:{}", random_id()?),
        };
        atomic_write(
            &path,
            &serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
        )?;
        value
    };
    let suffix = identity
        .bank_id
        .strip_prefix("ema-companion-")
        .ok_or("桌宠 bank 身份不合法")?;
    if suffix.len() != 36
        || !suffix.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        || !identity.owner.starts_with("traybuddy-ema-owner:")
    {
        return Err("桌宠 bank 身份不合法，拒绝接入".into());
    }
    Ok(identity)
}

pub struct Secrets(pub PathBuf);
impl Secrets {
    fn path(&self, kind: &str) -> Result<PathBuf, String> {
        if !["chat", "vision", "speech", "memory"].contains(&kind) {
            return Err("凭据类型无效".into());
        }
        Ok(self.0.join(format!("{kind}.key")))
    }
    pub fn has(&self, kind: &str) -> bool {
        self.path(kind).map(|p| p.exists()).unwrap_or(false)
    }
    pub fn set(&self, kind: &str, value: &str) -> Result<(), String> {
        if value.is_empty() {
            return Ok(());
        }
        atomic_write(&self.path(kind)?, &protect(value.as_bytes(), false)?)
    }
    pub fn get(&self, kind: &str) -> Result<String, String> {
        let path = self.path(kind)?;
        if !path.exists() {
            return Ok(String::new());
        }
        String::from_utf8(protect(&fs::read(path).map_err(|e| e.to_string())?, true)?)
            .map_err(|_| "凭据解密失败".into())
    }
}

#[cfg(windows)]
fn protect(bytes: &[u8], decrypt: bool) -> Result<Vec<u8>, String> {
    #[repr(C)]
    struct Blob {
        len: u32,
        data: *mut u8,
    }
    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            input: *const Blob,
            description: *const u16,
            entropy: *const Blob,
            reserved: *mut std::ffi::c_void,
            prompt: *mut std::ffi::c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
        fn CryptUnprotectData(
            input: *const Blob,
            description: *mut *mut u16,
            entropy: *const Blob,
            reserved: *mut std::ffi::c_void,
            prompt: *mut std::ffi::c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(ptr: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }
    let input = Blob {
        len: bytes.len() as u32,
        data: bytes.as_ptr() as *mut u8,
    };
    let mut output = Blob {
        len: 0,
        data: std::ptr::null_mut(),
    };
    // DPAPI is scoped to the signed-in Windows user; UI prompts are forbidden.
    let success = unsafe {
        if decrypt {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                1,
                &mut output,
            )
        } else {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                1,
                &mut output,
            )
        }
    };
    if success == 0 {
        return Err("Windows 凭据保护失败".into());
    }
    let result = unsafe { std::slice::from_raw_parts(output.data, output.len as usize).to_vec() };
    unsafe {
        LocalFree(output.data.cast());
    }
    Ok(result)
}
#[cfg(not(windows))]
fn protect(_: &[u8], _: bool) -> Result<Vec<u8>, String> {
    Err("此版本仅支持 Windows 安全凭据存储".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reply_length_is_unlimited_by_default_and_accepts_user_limits() {
        let mut value = Settings::default();
        assert_eq!(value.reply_max_chars, 0);
        for limit in [0, 1, 39, 181, 10000, usize::MAX] {
            value.reply_max_chars = limit;
            assert!(value.validate().is_ok());
        }
        let legacy: Settings = serde_json::from_str(r#"{"reply_max_chars":120}"#).unwrap();
        assert_eq!(legacy.reply_max_chars, 120);
    }
    #[test]
    fn identity_survives_restart_and_rejects_missing_owner_file() {
        let dir = tempfile::tempdir().unwrap();
        let first = load_identity(dir.path()).unwrap();
        assert_eq!(first.bank_id, load_identity(dir.path()).unwrap().bank_id);
        fs::write(dir.path().join("history.sqlite"), "").unwrap();
        fs::remove_file(dir.path().join("identity.json")).unwrap();
        assert!(load_identity(dir.path()).is_err());
    }
    #[test]
    fn no_foreign_bank_can_enter_settings() {
        assert!(serde_json::from_str::<Settings>(r#"{"bank_id":"coding-agent"}"#).is_err());
        assert!(
            validate_url("http://127.0.0.1:18088/v1/default/banks/coding-agent", true).is_err()
        );
        assert!(validate_url("https://example.com", true).is_err());
        assert!(validate_url("http://127.0.0.1:18088", true).is_ok());
    }
    #[test]
    fn corrupt_settings_are_never_silently_defaulted() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("settings.json"), "broken").unwrap();
        assert!(load_settings(dir.path()).is_err());
        assert_eq!(
            fs::read_to_string(dir.path().join("settings.json")).unwrap(),
            "broken"
        );
    }
    #[test]
    fn atomic_updates_replace_existing_files_and_keep_previous_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        atomic_write(&path, b"first").unwrap();
        atomic_write(&path, b"second").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
        assert_eq!(
            fs::read_to_string(path.with_extension("bak")).unwrap(),
            "first"
        );
    }
    #[cfg(windows)]
    #[test]
    fn credentials_roundtrip_without_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let keys = Secrets(dir.path().into());
        keys.set("chat", "test-secret-123").unwrap();
        assert_eq!(keys.get("chat").unwrap(), "test-secret-123");
        assert!(
            !String::from_utf8_lossy(&fs::read(keys.path("chat").unwrap()).unwrap())
                .contains("test-secret")
        );
    }
}
