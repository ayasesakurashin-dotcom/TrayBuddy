//! Telegram 远程通道集成模块
//!
//! 复用 DSH 集成（`dsh.rs`）的进程管理方式：隐藏终端启动 `dsh --profile telegram`
//! （通过 `start-telegram.ps1` 注入 bot token / allowFrom），由用户通过托盘菜单
//! 手动开启/关闭，绝不随桌宠开机自启。
//!
//! 与 DSH 服务器一样带健康监控：启动后通过 `getMe` 确认 bot token 有效才标记
//! Running，随后后台 watcher 持续检测进程存活与 token 有效性，避免"假启动"
//! （进程活着但 token 已失效，实际 401 无法收发消息）。

use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::modules::dsh::{dsh_log, refresh_tray_menu};

/// Windows `CREATE_NO_WINDOW`：隐藏启动终端
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Telegram 启动脚本的绝对路径：设置 env 并在前台运行 `dsh --profile telegram`。
/// 脚本内含 bot token 与 allowFrom，修改它们直接编辑该脚本即可。
const TELEGRAM_SCRIPT_PATH: &str = r"D:\dsh_channel\start-telegram.ps1";

/// 启动后等待 getMe 就绪的最大时长
const TG_READY_TIMEOUT_SECS: u64 = 30;
/// 启动阶段健康轮询间隔（毫秒）
const TG_HEALTH_POLL_MS: u64 = 2000;
/// 运行阶段 watcher 巡检间隔（秒）
const TG_WATCHER_INTERVAL_SECS: u64 = 10;

// ========================================================================= //
// 状态与数据结构
// ========================================================================= //

/// Telegram 通道生命周期状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TelegramState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Error,
}

/// 对外暴露的 Telegram 通道状态快照
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramStatus {
    pub state: TelegramState,
    pub last_error: Option<String>,
}

/// Telegram 通道管理器（由 AppState 持有）
pub struct TelegramManager {
    state: TelegramState,
    child: Option<std::process::Child>,
    last_error: Option<String>,
    /// 事件监控任务代数（保证同一时刻只有一个 watcher）
    watcher_seq: u64,
}

impl Default for TelegramManager {
    fn default() -> Self {
        Self {
            state: TelegramState::Stopped,
            child: None,
            last_error: None,
            watcher_seq: 0,
        }
    }
}

impl TelegramManager {
    pub fn status(&self) -> TelegramStatus {
        TelegramStatus {
            state: self.state,
            last_error: self.last_error.clone(),
        }
    }

    fn set_state(&mut self, state: TelegramState, error: Option<String>) {
        self.state = state;
        if error.is_some() {
            self.last_error = error;
        }
        if state == TelegramState::Stopped || state == TelegramState::Error {
            self.child = None;
        }
    }

    /// 终止进程树（Windows 使用 taskkill /T）
    fn kill_tree(child: &mut std::process::Child) {
        let pid = child.id();
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = child.kill();
        }
        let _ = child.wait();
    }
}

// ========================================================================= //
// token 读取与健康检查
// ========================================================================= //

/// 从启动脚本解析 `$BOT_TOKEN = "..."`（token 仍只存于脚本，用户可直接改）。
fn read_token_from_script() -> Option<String> {
    let content = std::fs::read_to_string(TELEGRAM_SCRIPT_PATH).ok()?;
    for line in content.lines() {
        let t = line.trim();
        let Some(rest) = t.strip_prefix("$BOT_TOKEN") else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim();
        if rest.len() >= 2 && rest.starts_with('"') && rest.ends_with('"') {
            return Some(rest[1..rest.len() - 1].to_string());
        }
    }
    None
}

/// getMe 健康检查：Ok(true)=200 有效；Ok(false)=被拒（401/revoke）；Err=网络不可达。
async fn telegram_health(token: &str) -> Result<bool, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("https://api.telegram.org/bot{}/getMe", token);
    match client.get(&url).send().await {
        Ok(resp) => Ok(resp.status().is_success()),
        Err(e) => Err(e.to_string()),
    }
}

// ========================================================================= //
// 启动 / 停止
// ========================================================================= //

/// 启动 Telegram 远程通道（幂等；已运行则直接返回）。
/// 先 spawn 进程标记 Starting，再后台 getMe 确认有效后标记 Running。
pub fn start_telegram(app: AppHandle) -> Result<TelegramStatus, String> {
    let state: tauri::State<crate::app_state::AppState> = app.state();
    let mut mgr = state.telegram.lock().unwrap();

    match mgr.state {
        TelegramState::Running | TelegramState::Starting => return Ok(mgr.status()),
        TelegramState::Stopping => return Err("telegram channel is stopping, please wait".into()),
        _ => {}
    }

    // 若记录的 child 仍在运行（状态与进程短暂不同步），视为运行中
    if let Some(child) = mgr.child.as_mut() {
        if matches!(child.try_wait(), Ok(None)) {
            mgr.set_state(TelegramState::Running, None);
            let status = mgr.status();
            drop(mgr);
            refresh_tray_menu(&app);
            return Ok(status);
        }
        mgr.child = None;
    }

    if !std::path::Path::new(TELEGRAM_SCRIPT_PATH).exists() {
        let err = format!("telegram startup script not found: {}", TELEGRAM_SCRIPT_PATH);
        mgr.set_state(TelegramState::Error, Some(err.clone()));
        let status = mgr.status();
        drop(mgr);
        dsh_log(&app, &format!("telegram start failed: {}", err));
        refresh_tray_menu(&app);
        return Err(err);
    }
    if read_token_from_script().is_none() {
        let err = "telegram token not found in startup script ($BOT_TOKEN)".to_string();
        mgr.set_state(TelegramState::Error, Some(err.clone()));
        let status = mgr.status();
        drop(mgr);
        dsh_log(&app, &format!("telegram start failed: {}", err));
        refresh_tray_menu(&app);
        return Err(err);
    }

    let mut cmd = std::process::Command::new("cmd");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.args([
        "/C", "powershell", "-NoProfile", "-ExecutionPolicy", "Bypass",
        "-File", TELEGRAM_SCRIPT_PATH,
    ]);

    match cmd.spawn() {
        Ok(child) => {
            mgr.child = Some(child);
            mgr.set_state(TelegramState::Starting, None);
            let status = mgr.status();
            drop(mgr);

            dsh_log(&app, "spawned dsh --profile telegram (hidden)");
            refresh_tray_menu(&app);

            // 后台等待 getMe 就绪
            let app2 = app.clone();
            tauri::async_runtime::spawn(async move {
                wait_until_ready(app2).await;
            });

            Ok(status)
        }
        Err(e) => {
            let err = format!("failed to spawn telegram channel: {}", e);
            mgr.set_state(TelegramState::Error, Some(err.clone()));
            let status = mgr.status();
            drop(mgr);
            dsh_log(&app, &format!("telegram start failed: {}", err));
            refresh_tray_menu(&app);
            Err(err)
        }
    }
}

/// 轮询 getMe 直到就绪；成功标记 Running 并启动 watcher。
async fn wait_until_ready(app: AppHandle) {
    use tokio::time::{sleep, Instant};

    let deadline = Instant::now() + Duration::from_secs(TG_READY_TIMEOUT_SECS);
    loop {
        // 子进程提前退出
        let child_exited = {
            let state: tauri::State<crate::app_state::AppState> = app.state();
            let mut mgr = state.telegram.lock().unwrap();
            if let Some(child) = mgr.child.as_mut() {
                matches!(child.try_wait(), Ok(Some(_)))
            } else {
                false
            }
        };
        if child_exited {
            let status = {
                let state: tauri::State<crate::app_state::AppState> = app.state();
                let mut mgr = state.telegram.lock().unwrap();
                mgr.set_state(
                    TelegramState::Error,
                    Some("telegram channel process exited during startup".into()),
                );
                mgr.status()
            };
            dsh_log(&app, "telegram channel exited during startup");
            let _ = status;
            refresh_tray_menu(&app);
            return;
        }

        let Some(token) = read_token_from_script() else {
            let status = {
                let state: tauri::State<crate::app_state::AppState> = app.state();
                let mut mgr = state.telegram.lock().unwrap();
                mgr.set_state(TelegramState::Error, Some("telegram token not found".into()));
                mgr.status()
            };
            let _ = status;
            refresh_tray_menu(&app);
            return;
        };

        match telegram_health(&token).await {
            Ok(true) => {
                {
                    let state: tauri::State<crate::app_state::AppState> = app.state();
                    let mut mgr = state.telegram.lock().unwrap();
                    if mgr.state == TelegramState::Starting || mgr.state == TelegramState::Stopped {
                        mgr.set_state(TelegramState::Running, None);
                    }
                }
                dsh_log(&app, "telegram channel ready (getMe ok)");
                refresh_tray_menu(&app);
                start_watcher(app);
                return;
            }
            Ok(false) => {
                let status = {
                    let state: tauri::State<crate::app_state::AppState> = app.state();
                    let mut mgr = state.telegram.lock().unwrap();
                    mgr.set_state(
                        TelegramState::Error,
                        Some("telegram token rejected (401): it was revoked or is wrong".into()),
                    );
                    mgr.status()
                };
                dsh_log(&app, "telegram token rejected (401)");
                let _ = status;
                refresh_tray_menu(&app);
                return;
            }
            Err(_) => {
                // 网络暂时不可达，继续重试直到超时
            }
        }

        if Instant::now() >= deadline {
            let status = {
                let state: tauri::State<crate::app_state::AppState> = app.state();
                let mut mgr = state.telegram.lock().unwrap();
                mgr.set_state(
                    TelegramState::Error,
                    Some(format!("telegram did not become ready within {}s", TG_READY_TIMEOUT_SECS)),
                );
                mgr.status()
            };
            dsh_log(&app, "telegram startup timed out");
            let _ = status;
            refresh_tray_menu(&app);
            return;
        }

        sleep(Duration::from_millis(TG_HEALTH_POLL_MS)).await;
    }
}

// ========================================================================= //
// 运行期监控（进程存活 + token 有效性）
// ========================================================================= //

/// 启动运行期 watcher（代数机制保证同一时刻只有一个）。
pub fn start_watcher(app: AppHandle) {
    let seq = {
        let state: tauri::State<crate::app_state::AppState> = app.state();
        let mut mgr = state.telegram.lock().unwrap();
        mgr.watcher_seq += 1;
        mgr.watcher_seq
    };
    tauri::async_runtime::spawn(async move {
        watcher_loop(app, seq).await;
    });
}

async fn watcher_loop(app: AppHandle, seq: u64) {
    loop {
        let current_seq = {
            let state: tauri::State<crate::app_state::AppState> = app.state();
            let mgr = state.telegram.lock().unwrap();
            mgr.watcher_seq
        };
        if current_seq != seq {
            return;
        }

        // 1) 进程存活检查
        let child_exited = {
            let state: tauri::State<crate::app_state::AppState> = app.state();
            let mut mgr = state.telegram.lock().unwrap();
            if let Some(child) = mgr.child.as_mut() {
                matches!(child.try_wait(), Ok(Some(_)))
            } else {
                false
            }
        };
        if child_exited {
            let status = {
                let state: tauri::State<crate::app_state::AppState> = app.state();
                let mut mgr = state.telegram.lock().unwrap();
                mgr.set_state(TelegramState::Stopped, None);
                mgr.status()
            };
            dsh_log(&app, "telegram channel process exited");
            let _ = status;
            refresh_tray_menu(&app);
            return;
        }

        // 2) token 有效性检查（检测 revoke / 换 bot）
        if let Some(token) = read_token_from_script() {
            if let Ok(false) = telegram_health(&token).await {
                let status = {
                    let state: tauri::State<crate::app_state::AppState> = app.state();
                    let mut mgr = state.telegram.lock().unwrap();
                    mgr.set_state(
                        TelegramState::Error,
                        Some("telegram token rejected (401): it was revoked or is wrong".into()),
                    );
                    mgr.status()
                };
                dsh_log(&app, "telegram token became invalid (401) during run");
                let _ = status;
                refresh_tray_menu(&app);
                return;
            }
        }

        tokio::time::sleep(Duration::from_secs(TG_WATCHER_INTERVAL_SECS)).await;
    }
}

/// 停止 Telegram 远程通道
pub fn stop_telegram(app: AppHandle) -> Result<TelegramStatus, String> {
    let state: tauri::State<crate::app_state::AppState> = app.state();
    let mut mgr = state.telegram.lock().unwrap();

    if mgr.state == TelegramState::Stopped || mgr.state == TelegramState::Error {
        mgr.set_state(TelegramState::Stopped, None);
        let status = mgr.status();
        drop(mgr);
        refresh_tray_menu(&app);
        return Ok(status);
    }

    mgr.set_state(TelegramState::Stopping, None);
    if let Some(mut child) = mgr.child.take() {
        dsh_log(&app, "stopping telegram channel");
        TelegramManager::kill_tree(&mut child);
    }
    mgr.set_state(TelegramState::Stopped, None);
    let status = mgr.status();
    drop(mgr);
    dsh_log(&app, "telegram channel stopped");
    refresh_tray_menu(&app);
    Ok(status)
}
