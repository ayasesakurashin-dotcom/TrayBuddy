//! DeepSeek Harness (DSH) 集成命令
//!
//! 提供给前端/调试页面的 IPC 命令：
//! - `dsh_get_status` — 查询 DSH 状态
//! - `dsh_start` / `dsh_stop` — 启动/停止 DSH 服务器（隐藏终端，不自动开浏览器）
//! - `dsh_toggle_panel` — 在桌宠旁弹出 DSH 交互面板（完整 Web UI）
//! - `dsh_open_browser` — 在默认浏览器打开 DSH
//! - `dsh_get_assets` — 返回打包的完成提示音资源路径（供桌宠播放）

use tauri::{AppHandle, State};

use crate::app_state::AppState;
use crate::modules::dsh;

/// 查询 DSH 集成状态
#[tauri::command]
pub fn dsh_get_status(state: State<'_, AppState>) -> dsh::DshStatus {
    let mgr = state.dsh.lock().unwrap();
    mgr.status()
}

/// 启动 DSH（幂等；已运行则直接接入）
#[tauri::command]
pub fn dsh_start(app: AppHandle) -> Result<dsh::DshStatus, String> {
    dsh::start_dsh(app)
}

/// 停止 DSH（仅终止由本程序启动的实例）
#[tauri::command]
pub fn dsh_stop(app: AppHandle) -> Result<dsh::DshStatus, String> {
    dsh::stop_dsh(app)
}

/// 打开/收起 DSH 交互面板（停靠桌宠旁）
#[tauri::command]
pub fn dsh_toggle_panel(app: AppHandle) -> Result<(), String> {
    dsh::toggle_dsh_panel(&app)
}

/// 在默认浏览器中打开 DSH Web UI
#[tauri::command]
pub fn dsh_open_browser(app: AppHandle) -> Result<(), String> {
    dsh::open_dsh_in_browser(&app)
}

/// 完成提示音等打包资源路径
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DshAssets {
    /// 任务完成提示音（樱羽艾玛素材）绝对路径；不存在时为空字符串
    pub completed_audio: String,
}

/// 获取随应用打包的 DSH 音频资源绝对路径
#[tauri::command]
pub fn dsh_get_assets(app: AppHandle) -> DshAssets {
    DshAssets {
        completed_audio: dsh::resolve_dsh_asset_path(&app, "ema_amazing.ogg"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsh_assets_serialize_camel_case() {
        let a = DshAssets {
            completed_audio: "C:/x/ema_amazing.ogg".into(),
        };
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"completedAudio\""));
    }
}
