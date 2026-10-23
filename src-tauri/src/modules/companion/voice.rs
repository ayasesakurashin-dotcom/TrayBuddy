use super::{character, providers, settings, Companion};
use crate::app_state::AppState;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::Manager;

#[derive(Serialize)]
pub struct Sample {
    pub name: String,
    pub candidate_text: String,
}
pub fn samples(app: &tauri::AppHandle) -> Result<Vec<Sample>, String> {
    let state = app.state::<AppState>();
    let cfg = app.state::<Companion>().settings.lock().unwrap().clone();
    let profile = character::resolve(app, &cfg);
    let rm = state.resource_manager.lock().unwrap();
    let info = rm.current_mod.as_ref().ok_or("请先加载角色")?;
    let locale = asset_locale(&profile.voice_locale, &info.manifest.default_audio_lang_id);
    Ok(info
        .audios
        .get(locale)
        .map(|list| {
            list.iter()
                .filter(|a| a.audio.ends_with(".wav"))
                .map(|a| Sample {
                    name: a.name.to_string(),
                    candidate_text: rm
                        .get_text_by_name(locale, &a.name)
                        .map(|t| t.text.to_string())
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default())
}
pub fn read(app: &tauri::AppHandle, name: &str) -> Result<Vec<u8>, String> {
    let state = app.state::<AppState>();
    let cfg = app.state::<Companion>().settings.lock().unwrap().clone();
    let profile = character::resolve(app, &cfg);
    let (base, relative) = {
        let rm = state.resource_manager.lock().unwrap();
        let info = rm.current_mod.as_ref().ok_or("请先加载角色")?;
        if info.manifest.id.as_ref() != profile.id {
            return Err("当前角色与陪伴配置不一致".into());
        }
        let locale = asset_locale(&profile.voice_locale, &info.manifest.default_audio_lang_id);
        let audio = rm
            .get_audio_by_name(locale, name)
            .ok_or("未找到所选语音素材")?;
        (info.path.clone(), format!("audio/{}", audio.audio))
    };
    let path = std::path::Path::new(&relative);
    if path
        .components()
        .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err("语音素材路径无效".into());
    }
    let bytes = if base.to_string_lossy().starts_with("tbuddy-archive://") {
        state
            .archive_store
            .lock()
            .unwrap()
            .read_file(&profile.id, &relative)?
    } else {
        let root = base.canonicalize().map_err(|_| "素材目录不存在")?;
        let file = root
            .join(path)
            .canonicalize()
            .map_err(|_| "语音素材不存在")?;
        if !file.starts_with(&root) {
            return Err("语音素材超出角色目录".into());
        }
        if std::fs::metadata(&file).map_err(|e| e.to_string())?.len() > 8_000_000 {
            return Err("单段语音素材过大".into());
        }
        std::fs::read(file).map_err(|e| e.to_string())?
    };
    if bytes.len() < 44
        || bytes.len() > 8_000_000
        || &bytes[..4] != b"RIFF"
        || &bytes[8..12] != b"WAVE"
    {
        return Err("请选择有效的 WAV 语音素材".into());
    }
    Ok(bytes)
}
pub async fn create(app: &tauri::AppHandle, names: Vec<String>) -> Result<String, String> {
    if names.is_empty() || names.len() > 8 {
        return Err("请选择 1—8 段清晰的语音素材".into());
    }
    let service = app.state::<Companion>();
    service.ensure_ready()?;
    let mut cfg = service.settings.lock().unwrap().clone();
    cfg.validate()?;
    let role = character::resolve(app, &cfg);
    let key = service.secrets.get("speech")?;
    if key.is_empty() {
        return Err("请先保存 Fish Audio API 密钥".into());
    }
    let mut form = reqwest::multipart::Form::new()
        .text("type", "tts")
        .text(
            "title",
            format!("{} companion private voice", role.display_name),
        )
        .text("train_mode", "fast")
        .text("visibility", "private")
        .text("enhance_audio_quality", "true")
        .text("generate_sample", "false");
    let mut total = 0;
    let mut provenance = Vec::new();
    for name in &names {
        if character::current_mod(app) != role.id {
            return Err("角色已切换，已停止上传参考素材".into());
        }
        let data = read(app, name)?;
        total += data.len();
        if total > 20_000_000 {
            return Err("参考素材总量请控制在 20MB 内".into());
        }
        provenance.push(json!({"sample":name,"sha256":format!("{:x}",Sha256::digest(&data))}));
        form = form.part(
            "voices",
            reqwest::multipart::Part::bytes(data)
                .file_name(format!("{name}.wav"))
                .mime_str("audio/wav")
                .map_err(|e| e.to_string())?,
        );
    }
    // No guessed transcripts: provider ASR handles the actual recordings.
    let response = providers::client(false)?
        .post(format!(
            "{}/model",
            cfg.speech.base_url.trim_end_matches('/')
        ))
        .bearer_auth(key)
        .multipart(form)
        .send()
        .await
        .map_err(|_| "私有声音创建连接失败")?;
    if !response.status().is_success() {
        return Err(format!(
            "私有声音创建失败（HTTP {}）",
            response.status().as_u16()
        ));
    }
    let data: Value = response.json().await.map_err(|_| "声音模型响应无效")?;
    if data.get("visibility").and_then(Value::as_str) != Some("private") {
        return Err("服务未确认声音为私有，未自动采用该模型；请到 Fish Audio 检查".into());
    }
    let id = data
        .get("_id")
        .or_else(|| data.get("id"))
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or("服务未返回声音 ID")?
        .to_string();
    settings::atomic_write(&service.dir.join("voice-provenance.json"),&serde_json::to_vec_pretty(&json!({"voice_id":id,"samples":provenance,"created_at":chrono::Utc::now().to_rfc3339(),"transcription":"provider-asr"})).map_err(|e|e.to_string())?)?;
    // Reload settings so an in-flight upload cannot undo concurrent menu changes.
    cfg = service.settings.lock().unwrap().clone();
    if character::profile_for_mod(&role.id).is_some() {
        cfg.voice_id = id.clone();
    }
    let entry = cfg
        .characters
        .entry(role.id.clone())
        .or_insert_with(|| character::RoleSettings {
            display_name: role.display_name.clone(),
            persona: role.persona.clone(),
            voice_locale: role.voice_locale.clone(),
            voice_id: String::new(),
        });
    entry.voice_id = id.clone();
    service.save(cfg)?;
    Ok(id)
}

fn asset_locale<'a>(locale: &'a str, fallback: &'a str) -> &'a str {
    match locale {
        "original" => fallback,
        "ja" => "jp",
        other => other,
    }
}
