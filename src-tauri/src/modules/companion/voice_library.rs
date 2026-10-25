//! Local, persistent reference audio. This module never calls a provider.
use super::settings;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
};

pub const MAX_SAMPLES: usize = 8;
pub const MAX_SAMPLE_BYTES: usize = 8_000_000;
pub const MAX_TOTAL_BYTES: usize = 20_000_000;
static LIBRARY_GATE: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalSample {
    pub id: String,
    pub name: String,
    pub size: usize,
    pub mime_type: String,
}

pub fn audio_type(bytes: &[u8]) -> Result<&'static str, String> {
    if bytes.len() > MAX_SAMPLE_BYTES {
        return Err("单段语音素材须在 8MB 内".into());
    }
    if bytes.len() >= 44 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        return Ok("audio/wav");
    }
    // ID3 metadata or a valid MPEG audio frame header (not an AAC ADTS header).
    if bytes.len() >= 10
        && (&bytes[..3] == b"ID3"
            || (bytes[0] == 0xff
                && bytes[1] & 0xe0 == 0xe0
                && bytes[1] & 0x18 != 0x08
                && bytes[1] & 0x06 != 0
                && bytes[2] & 0xf0 != 0
                && bytes[2] & 0xf0 != 0xf0
                && bytes[2] & 0x0c != 0x0c))
    {
        return Ok("audio/mpeg");
    }
    Err("请选择有效的 WAV 或 MP3 语音文件".into())
}

fn directory(root: &Path, role_id: &str) -> Result<PathBuf, String> {
    if role_id.is_empty() || role_id.len() > 100 {
        return Err("角色标识无效".into());
    }
    Ok(root
        .join("voice-samples")
        .join(format!("{:x}", Sha256::digest(role_id.as_bytes()))))
}

fn read_limited(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|_| "无法读取语音素材，请重新导入")?;
    let info = file.metadata().map_err(|e| e.to_string())?;
    if !info.is_file() || info.len() > limit as u64 {
        return Err("语音素材大小或文件类型无效".into());
    }
    let mut data = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() > limit {
        return Err("语音素材过大".into());
    }
    Ok(data)
}

fn load(dir: &Path) -> Result<Vec<LocalSample>, String> {
    let path = dir.join("samples.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let samples: Vec<LocalSample> = serde_json::from_slice(&read_limited(&path, 65_536)?)
        .map_err(|_| "本地音声列表损坏，原文件已保留")?;
    let mut ids = std::collections::HashSet::new();
    if samples.len() > MAX_SAMPLES
        || samples.iter().any(|s| {
            s.id.len() != 64
                || !s
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || !ids.insert(s.id.as_str())
                || s.name.is_empty()
                || s.name.chars().count() > 255
                || s.size == 0
                || s.size > MAX_SAMPLE_BYTES
                || !matches!(s.mime_type.as_str(), "audio/wav" | "audio/mpeg")
        })
        || samples.iter().map(|s| s.size).sum::<usize>() > MAX_TOTAL_BYTES
    {
        return Err("本地音声列表无效，原文件已保留".into());
    }
    Ok(samples)
}

fn save(dir: &Path, samples: &[LocalSample]) -> Result<(), String> {
    settings::atomic_write(
        &dir.join("samples.json"),
        &serde_json::to_vec_pretty(samples).map_err(|e| e.to_string())?,
    )
}

pub fn list(root: &Path, role_id: &str) -> Result<Vec<LocalSample>, String> {
    let _gate = LIBRARY_GATE.lock().unwrap();
    load(&directory(root, role_id)?)
}

pub fn import(root: &Path, role_id: &str, paths: &[PathBuf]) -> Result<Vec<LocalSample>, String> {
    let _gate = LIBRARY_GATE.lock().unwrap();
    if paths.is_empty() || paths.len() > MAX_SAMPLES {
        return Err("每次请选择 1—8 段音声".into());
    }
    let dir = directory(root, role_id)?;
    let mut samples = load(&dir)?;
    let mut additions = Vec::new();
    for path in paths {
        let bytes = read_limited(path, MAX_SAMPLE_BYTES)?;
        let mime_type = audio_type(&bytes)?.to_string();
        let id = format!("{:x}", Sha256::digest(&bytes));
        if samples.iter().any(|s| s.id == id) {
            continue;
        }
        let name: String = path
            .file_name()
            .ok_or("语音文件名无效")?
            .to_string_lossy()
            .chars()
            .take(255)
            .collect();
        samples.push(LocalSample {
            id: id.clone(),
            name,
            size: bytes.len(),
            mime_type,
        });
        if samples.len() > MAX_SAMPLES
            || samples.iter().map(|s| s.size).sum::<usize>() > MAX_TOTAL_BYTES
        {
            return Err("每个角色最多暂存 8 段音声，总量须在 20MB 内；可先移除旧素材".into());
        }
        additions.push((id, bytes));
    }
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (id, bytes) in &additions {
        settings::atomic_write(&dir.join(format!("{id}.audio")), bytes)?;
    }
    save(&dir, &samples)?;
    Ok(samples)
}

pub fn read(root: &Path, role_id: &str, id: &str) -> Result<(LocalSample, Vec<u8>), String> {
    let _gate = LIBRARY_GATE.lock().unwrap();
    let dir = directory(root, role_id)?;
    let sample = load(&dir)?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or("未找到当前角色的暂存音声")?;
    let root = dir.canonicalize().map_err(|_| "音声暂存目录不存在")?;
    let file = root
        .join(format!("{}.audio", sample.id))
        .canonicalize()
        .map_err(|_| "暂存音声不存在")?;
    if !file.starts_with(&root) {
        return Err("音声路径超出暂存目录".into());
    }
    let bytes = read_limited(&file, MAX_SAMPLE_BYTES)?;
    if bytes.len() != sample.size
        || audio_type(&bytes)? != sample.mime_type
        || format!("{:x}", Sha256::digest(&bytes)) != sample.id
    {
        return Err("暂存音声已变化，请移除后重新导入".into());
    }
    Ok((sample, bytes))
}

pub fn remove(root: &Path, role_id: &str, id: &str) -> Result<Vec<LocalSample>, String> {
    let _gate = LIBRARY_GATE.lock().unwrap();
    let dir = directory(root, role_id)?;
    let mut samples = load(&dir)?;
    let index = samples
        .iter()
        .position(|s| s.id == id)
        .ok_or("未找到当前角色的暂存音声")?;
    let sample = samples.remove(index);
    save(&dir, &samples)?;
    let path = dir.join(format!("{}.audio", sample.id));
    if path.exists() {
        fs::remove_file(path).map_err(|_| "已从列表移除，暂存文件清理失败")?;
    }
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wav() -> Vec<u8> {
        let mut data = vec![0u8; 48];
        data[..4].copy_from_slice(b"RIFF");
        data[8..12].copy_from_slice(b"WAVE");
        data
    }
    #[test]
    fn import_survives_reopen_deduplicates_and_isolates_roles() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("sample.wav");
        fs::write(&source, wav()).unwrap();
        let first = import(dir.path(), "role-a", &[source.clone()]).unwrap();
        fs::remove_file(&source).unwrap();
        assert_eq!(list(dir.path(), "role-a").unwrap()[0].id, first[0].id);
        assert_eq!(read(dir.path(), "role-a", &first[0].id).unwrap().1, wav());
        assert!(list(dir.path(), "role-b").unwrap().is_empty());
        assert!(read(dir.path(), "role-b", &first[0].id).is_err());
        fs::write(&source, wav()).unwrap();
        assert_eq!(import(dir.path(), "role-a", &[source]).unwrap().len(), 1);
        assert!(remove(dir.path(), "role-a", "../outside").is_err());
        assert!(remove(dir.path(), "role-a", &first[0].id)
            .unwrap()
            .is_empty());
        assert!(read(dir.path(), "role-a", &first[0].id).is_err());
    }
    #[test]
    fn invalid_batch_and_tampered_files_never_replace_saved_audio() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.wav");
        fs::write(&good, wav()).unwrap();
        let bad = dir.path().join("bad.wav");
        fs::write(&bad, b"not audio").unwrap();
        assert!(import(dir.path(), "role", &[good.clone(), bad]).is_err());
        assert!(list(dir.path(), "role").unwrap().is_empty());
        let samples = import(dir.path(), "role", &[good]).unwrap();
        fs::write(
            directory(dir.path(), "role")
                .unwrap()
                .join(format!("{}.audio", samples[0].id)),
            b"tampered",
        )
        .unwrap();
        assert!(read(dir.path(), "role", &samples[0].id).is_err());
    }
    #[test]
    fn size_count_and_audio_format_are_checked() {
        assert!(audio_type(&vec![0; MAX_SAMPLE_BYTES + 1]).is_err());
        assert_eq!(
            audio_type(b"ID3\x04\0\0\0\0\0\0sound").unwrap(),
            "audio/mpeg"
        );
        assert!(audio_type(b"\xff\xf1\x50\x80invalid AAC").is_err());
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<_> = (0..9)
            .map(|i| {
                let path = dir.path().join(format!("{i}.wav"));
                let mut data = wav();
                data[47] = i;
                fs::write(&path, data).unwrap();
                path
            })
            .collect();
        assert_eq!(import(dir.path(), "role", &paths[..8]).unwrap().len(), 8);
        assert!(import(dir.path(), "role", &paths[8..]).is_err());
        assert_eq!(list(dir.path(), "role").unwrap().len(), 8);
    }
}
