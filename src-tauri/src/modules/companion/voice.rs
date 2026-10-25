pub use super::voice_library::LocalSample;
use super::{character, providers, settings, voice_library, Companion};
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
pub fn ensure_role(app: &tauri::AppHandle, role_id: &str) -> Result<(), String> {
    app.state::<Companion>().ensure_ready()?;
    if character::current_mod(app) != role_id {
        return Err("角色已切换，请重新打开当前角色的声音素材".into());
    }
    Ok(())
}

pub fn local_samples(app: &tauri::AppHandle, role_id: &str) -> Result<Vec<LocalSample>, String> {
    ensure_role(app, role_id)?;
    voice_library::list(&app.state::<Companion>().dir, role_id)
}

pub fn import_local(
    app: &tauri::AppHandle,
    role_id: &str,
    paths: &[std::path::PathBuf],
) -> Result<Vec<LocalSample>, String> {
    ensure_role(app, role_id)?;
    voice_library::import(&app.state::<Companion>().dir, role_id, paths)
}

pub fn read_local(
    app: &tauri::AppHandle,
    role_id: &str,
    id: &str,
) -> Result<(LocalSample, Vec<u8>), String> {
    ensure_role(app, role_id)?;
    voice_library::read(&app.state::<Companion>().dir, role_id, id)
}

pub fn remove_local(
    app: &tauri::AppHandle,
    role_id: &str,
    id: &str,
) -> Result<Vec<LocalSample>, String> {
    ensure_role(app, role_id)?;
    voice_library::remove(&app.state::<Companion>().dir, role_id, id)
}

pub fn samples(app: &tauri::AppHandle, role_id: &str) -> Result<Vec<Sample>, String> {
    ensure_role(app, role_id)?;
    let state = app.state::<AppState>();
    let cfg = app.state::<Companion>().settings.lock().unwrap().clone();
    let profile = character::resolve(app, &cfg);
    let rm = state.resource_manager.lock().unwrap();
    let info = rm.current_mod.as_ref().ok_or("请先加载角色")?;
    if info.manifest.id.as_ref() != role_id {
        return Err("角色素材正在切换，请稍后重试".into());
    }
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
pub fn read(app: &tauri::AppHandle, role_id: &str, name: &str) -> Result<Vec<u8>, String> {
    ensure_role(app, role_id)?;
    let state = app.state::<AppState>();
    let cfg = app.state::<Companion>().settings.lock().unwrap().clone();
    let profile = character::resolve(app, &cfg);
    let (base, relative) = {
        let rm = state.resource_manager.lock().unwrap();
        let info = rm.current_mod.as_ref().ok_or("请先加载角色")?;
        if info.manifest.id.as_ref() != role_id || profile.id != role_id {
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
pub async fn create(
    app: &tauri::AppHandle,
    role_id: String,
    names: Vec<String>,
    local_ids: Vec<String>,
) -> Result<String, String> {
    ensure_role(app, &role_id)?;
    if names.len() + local_ids.len() == 0
        || names.len() + local_ids.len() > voice_library::MAX_SAMPLES
    {
        return Err("请选择 1—8 段清晰的语音素材".into());
    }
    let service = app.state::<Companion>();
    service.ensure_ready()?;
    let cfg = service.settings.lock().unwrap().clone();
    cfg.validate()?;
    let role = character::resolve(app, &cfg);
    if role.id != role_id {
        return Err("角色已切换，请重新选择素材".into());
    }
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
    let mut seen = std::collections::HashSet::new();
    for name in &names {
        if !seen.insert(format!("preset:{name}")) {
            return Err("请勿重复选择语音素材".into());
        }
        let data = read(app, &role_id, name)?;
        total += data.len();
        if total > voice_library::MAX_TOTAL_BYTES {
            return Err("参考素材总量请控制在 20MB 内".into());
        }
        provenance.push(
            json!({"source":"preset","sample":name,"sha256":format!("{:x}",Sha256::digest(&data))}),
        );
        form = form.part(
            "voices",
            reqwest::multipart::Part::bytes(data)
                .file_name(format!("preset-{}.wav", provenance.len()))
                .mime_str("audio/wav")
                .map_err(|e| e.to_string())?,
        );
    }
    for id in &local_ids {
        if !seen.insert(format!("local:{id}")) {
            return Err("请勿重复选择语音素材".into());
        }
        let (sample, data) = read_local(app, &role_id, id)?;
        total += data.len();
        if total > voice_library::MAX_TOTAL_BYTES {
            return Err("参考素材总量请控制在 20MB 内".into());
        }
        let extension = if sample.mime_type == "audio/wav" {
            "wav"
        } else {
            "mp3"
        };
        provenance.push(json!({"source":"local","sample":sample.name,"sha256":sample.id}));
        form = form.part(
            "voices",
            reqwest::multipart::Part::bytes(data)
                .file_name(format!("local-{}.{extension}", provenance.len()))
                .mime_str(&sample.mime_type)
                .map_err(|e| e.to_string())?,
        );
    }
    ensure_role(app, &role_id)?;
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
    let provenance_dir = service.dir.join("voice-provenance");
    std::fs::create_dir_all(&provenance_dir).map_err(|e| e.to_string())?;
    settings::atomic_write(&provenance_dir.join(format!("{}.json", settings::random_id()?)),&serde_json::to_vec_pretty(&json!({"role_id":role_id,"voice_id":id,"samples":provenance,"created_at":chrono::Utc::now().to_rfc3339(),"transcription":"provider-asr"})).map_err(|e|e.to_string())?)?;
    bind_voice(&service, &role, &id)?;
    Ok(id)
}

fn bind_voice(
    service: &Companion,
    role: &character::RuntimeProfile,
    id: &str,
) -> Result<(), String> {
    // Serialize the entire read/modify/write, preserving concurrent settings and
    // binding to the original role even if the user switched during the request.
    let _gate = service.save_gate.lock().unwrap();
    service.ensure_ready()?;
    let mut cfg = service.settings.lock().unwrap().clone();
    let latest = character::resolve_for(&role.id, &role.display_name, &cfg);
    if latest.voice_id != role.voice_id {
        return Err(format!(
            "私有声音已建立（{id}），但该角色的声音设置已变化，未自动替换；可手动填写此 ID"
        ));
    }
    if character::uses_legacy_voice(&role.id) {
        cfg.voice_id = id.into();
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
    entry.voice_id = id.into();
    cfg.validate()?;
    settings::atomic_write(
        &service.dir.join("settings.json"),
        &serde_json::to_vec_pretty(&cfg).map_err(|e| e.to_string())?,
    )?;
    *service.settings.lock().unwrap() = cfg;
    service
        .settings_revision
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

fn asset_locale<'a>(locale: &'a str, fallback: &'a str) -> &'a str {
    match locale {
        "original" => fallback,
        "ja" => "jp",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn voice_binding_preserves_other_roles_shared_settings_and_memory_identity() {
        let dir = tempfile::tempdir().unwrap();
        let service = Companion::new(dir.path().into()).unwrap();
        let original = service.settings.lock().unwrap().clone();
        let role = character::resolve_for("other", "伙伴", &original);
        let identity = std::fs::read(dir.path().join("identity.json")).unwrap();
        let mut edited = original.clone();
        edited.nickname = "friend".into();
        edited.voice_id = "legacy-voice".into();
        edited.characters.insert(
            "third".into(),
            character::RoleSettings {
                voice_id: "third-voice".into(),
                ..Default::default()
            },
        );
        service.save(edited).unwrap();
        bind_voice(&service, &role, "uploaded-voice").unwrap();
        let saved = settings::load_settings(dir.path()).unwrap();
        assert_eq!(saved.characters["other"].voice_id, "uploaded-voice");
        assert_eq!(saved.characters["third"].voice_id, "third-voice");
        assert_eq!(saved.voice_id, "legacy-voice");
        assert_eq!(saved.nickname, "friend");
        assert_eq!(
            std::fs::read(dir.path().join("identity.json")).unwrap(),
            identity
        );
    }
    #[test]
    fn voice_binding_does_not_replace_a_voice_edited_while_uploading() {
        let dir = tempfile::tempdir().unwrap();
        let service = Companion::new(dir.path().into()).unwrap();
        let mut cfg = service.settings.lock().unwrap().clone();
        let role = character::resolve_for("ema", "艾玛", &cfg);
        cfg.characters.insert(
            "ema".into(),
            character::RoleSettings {
                voice_id: "manual-voice".into(),
                ..Default::default()
            },
        );
        service.save(cfg).unwrap();
        assert!(bind_voice(&service, &role, "uploaded-voice")
            .unwrap_err()
            .contains("uploaded-voice"));
        assert_eq!(
            settings::load_settings(dir.path()).unwrap().characters["ema"].voice_id,
            "manual-voice"
        );
    }
}
