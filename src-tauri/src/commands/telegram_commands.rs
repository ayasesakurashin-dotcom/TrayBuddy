//! Telegram 远程通道 IPC 命令
//!
//! - `telegram_get_status` — 查询通道状态
//! - `telegram_start` / `telegram_stop` — 启动/停止通道（隐藏终端）

use tauri::{AppHandle, State};

use crate::app_state::AppState;
use crate::modules::telegram::{self, TelegramStatus};

/// 查询 Telegram 通道状态
#[tauri::command]
pub fn telegram_get_status(state: State<'_, AppState>) -> TelegramStatus {
    let mgr = state.telegram.lock().unwrap();
    mgr.status()
}

/// 启动 Telegram 通道（幂等；已运行则直接返回）
#[tauri::command]
pub fn telegram_start(app: AppHandle) -> Result<TelegramStatus, String> {
    telegram::start_telegram(app)
}

/// 停止 Telegram 通道
#[tauri::command]
pub fn telegram_stop(app: AppHandle) -> Result<TelegramStatus, String> {
    telegram::stop_telegram(app)
}
