use serde::{de::DeserializeOwned, Serialize};
use std::path::Path;
pub fn load<T: DeserializeOwned + Default>(path: &Path) -> Result<T, String> {
    if !path.exists() {
        return Ok(T::default());
    }
    let bytes = std::fs::read(path).map_err(|_| "设置文件无法读取".to_string())?;
    if bytes.len() > 128 * 1024 {
        return Err("设置文件过大".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "设置文件损坏，已保留原文件".into())
}
pub fn save<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let parent = path.parent().ok_or("设置路径无效")?;
    std::fs::create_dir_all(parent).map_err(|_| "无法创建设置目录")?;
    let temp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| "设置无法编码")?;
    use std::io::Write;
    let mut file = std::fs::File::create(&temp).map_err(|_| "设置无法保存")?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "设置无法保存")?;
    if path.exists() {
        std::fs::copy(path, path.with_extension("json.bak")).map_err(|_| "设置备份失败")?;
    }
    std::fs::rename(&temp, path).map_err(|_| "设置替换失败".into())
}
