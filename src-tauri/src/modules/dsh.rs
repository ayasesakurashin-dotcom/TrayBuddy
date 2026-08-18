//! DeepSeek Harness (DSH) 集成模块
//!
//! 将 TrayBuddy 桌宠与 DeepSeek Harness 联动：
//!
//! - **启动/停止 DSH**：后台（隐藏终端）运行 `dsh web` 启动 DSH 服务器，
//!   不自动打开浏览器；仅当用户通过菜单/命令手动启动时才运行，
//!   绝不随桌宠开机自启（自启动功能保持原样，不修改）。
//! - **DSH 交互面板**：在桌宠旁边弹出 DSH 官方 Web UI 窗口
//!   （复用 DSH 自带的全部界面功能，含会话/工具/设置等）。
//! - **任务完成提醒**：通过 DSH 的 `host/session-status` 事件流
//!   （WebSocket `/api/events.host`）实时感知 Agent 运行状态，
//!   当所有 Agent 从运行转入空闲时判定“任务完成”，
//!   通知桌宠播放提示音（樱羽艾玛素材）并弹出气泡；
//!   出错时气泡提示失败。
//!
//! 通信协议参考 dshpet (dsh-kun-like-pet) 与 DSH 官方 web 前端：
//! - 健康检查：`GET http://127.0.0.1:{port}/`（200 即就绪）
//! - 状态流：WebSocket `ws://127.0.0.1:{port}/api/events.host`
//!   每帧为 `{"type":"server-request","rpcId":...,"method":...,"payload":{...}}`，
//!   其中 `payload.type` 为 `host/session-status` / `host/agent-error` 等。

#![allow(unused)]

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::modules::event_manager::{emit, events};

// ========================================================================= //
// 常量
// ========================================================================= //

/// DSH Web 服务器的默认端口（与 DSH web profile 默认值一致）
pub const DSH_DEFAULT_PORT: u16 = 3080;

/// DSH 交互面板窗口标签
pub const WINDOW_LABEL_DSH: &str = "dsh";

/// 启动后等待 DSH 就绪的最大时长（首次启动加载插件树较慢）
const DSH_READY_TIMEOUT_SECS: u64 = 90;

/// 健康检查轮询间隔（毫秒）
const DSH_HEALTH_POLL_MS: u64 = 1000;

/// 事件流断线后的重连间隔（毫秒）
const DSH_WS_RECONNECT_MS: u64 = 3000;

/// 判定“任务完成”的最短运行时长（毫秒）——过滤瞬时 flip
const DSH_MIN_RUN_MS: u64 = 1500;

/// 交互面板默认尺寸（适中大小，用户可缩放）
pub const DSH_PANEL_WIDTH: f64 = 480.0;
pub const DSH_PANEL_HEIGHT: f64 = 760.0;
pub const DSH_PANEL_MIN_WIDTH: f64 = 380.0;
pub const DSH_PANEL_MIN_HEIGHT: f64 = 480.0;

/// Windows `CREATE_NO_WINDOW`：隐藏启动终端
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ========================================================================= //
// 状态与数据结构
// ========================================================================= //

/// DSH 生命周期状态（序列化给前端）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DshState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Error,
}

impl DshState {
    pub fn as_str(&self) -> &'static str {
        match self {
            DshState::Stopped => "stopped",
            DshState::Starting => "starting",
            DshState::Running => "running",
            DshState::Stopping => "stopping",
            DshState::Error => "error",
        }
    }
}

/// 对外（前端）暴露的 DSH 状态快照
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DshStatus {
    pub state: DshState,
    /// 本程序是否持有 DSH 进程（false 表示连接到用户自行启动的实例）
    pub owned: bool,
    /// DSH Web UI 地址
    pub url: Option<String>,
    pub port: u16,
    pub last_error: Option<String>,
}

/// 单个 Agent 会话的运行追踪
struct SessionRun {
    running: bool,
    errored: bool,
    since: Instant,
}

/// DSH 集成管理器（由 AppState 持有）
pub struct DshManager {
    state: DshState,
    owned: bool,
    child: Option<std::process::Child>,
    port: u16,
    last_error: Option<String>,
    /// 当前处于 running 状态的会话集合（用于“全部空闲 ⇒ 任务完成”判定）
    running_sessions: HashSet<String>,
    session_runs: HashMap<String, SessionRun>,
    /// 事件监控任务代数（保证同一时刻只有一个 watcher，避免重复通知）
    watcher_seq: u64,
}

impl Default for DshManager {
    fn default() -> Self {
        // 端口可通过环境变量 DSH_PORT 覆盖（默认 3080，与 DSH web profile 一致）
        let port = std::env::var("DSH_PORT")
            .ok()
            .and_then(|v| v.trim().parse::<u16>().ok())
            .unwrap_or(DSH_DEFAULT_PORT);
        Self {
            state: DshState::Stopped,
            owned: false,
            child: None,
            port,
            last_error: None,
            running_sessions: HashSet::new(),
            session_runs: HashMap::new(),
            watcher_seq: 0,
        }
    }
}

impl DshManager {
    pub fn status(&self) -> DshStatus {
        DshStatus {
            state: self.state,
            owned: self.owned,
            url: match self.state {
                DshState::Running => Some(format!("http://127.0.0.1:{}/", self.port)),
                _ => None,
            },
            port: self.port,
            last_error: self.last_error.clone(),
        }
    }

    fn set_state(&mut self, state: DshState, error: Option<String>) {
        self.state = state;
        if error.is_some() {
            self.last_error = error;
        }
        if state == DshState::Stopped || state == DshState::Error {
            self.owned = false;
            self.child = None;
            self.running_sessions.clear();
            self.session_runs.clear();
        }
    }
}

// ========================================================================= //
// 日志（写入应用配置目录 dsh.log，便于诊断与自测）
// ========================================================================= //

/// 追加一行日志到 `{app_config_dir}/dsh.log`（>512KB 时截断保留尾部）
pub fn dsh_log(app: &AppHandle, msg: &str) {
    let Ok(dir) = app.path().app_config_dir() else {
        return;
    };
    let path = dir.join("dsh.log");
    let line = format!(
        "[{}] {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        msg
    );
    let _ = std::fs::create_dir_all(&dir);
    match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        Ok(mut f) => {
            use std::io::Write;
            let _ = f.write_all(line.as_bytes());
            if let Ok(meta) = f.metadata() {
                if meta.len() > 512 * 1024 {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let tail: String = content
                            .chars()
                            .rev()
                            .take(64 * 1024)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .collect();
                        let _ = std::fs::write(&path, tail);
                    }
                }
            }
        }
        Err(_) => {}
    }
}

// ========================================================================= //
// 进程命令解析
// ========================================================================= //

/// 在 PATH 中查找可执行文件（等价 `where`）
fn find_on_path(name: &str) -> Option<std::path::PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        for ext in ["", ".exe", ".cmd", ".bat"] {
            let candidate = dir.join(format!("{}{}", name, ext));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// 生成启动 DSH 的命令。
///
/// 依次尝试：
/// 1. PATH 中的 `dsh`（npm 全局安装的 shim，`cmd /C dsh web`）；
/// 2. `%APPDATA%\npm\dsh.cmd`；
/// 3. `node %APPDATA%\npm\node_modules\@deepseek-ai\dsh\lib\bin.js`。
fn resolve_dsh_command(port: u16) -> Option<std::process::Command> {
    let port_str = port.to_string();

    // 1) PATH 上的 dsh shim
    if find_on_path("dsh").is_some() {
        let mut cmd = std::process::Command::new("cmd");
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd.args(["/C", "dsh", "web", "--port", &port_str]);
        return Some(cmd);
    }

    // 2) %APPDATA%\npm\dsh.cmd
    if let Ok(appdata) = std::env::var("APPDATA") {
        let shim = std::path::Path::new(&appdata).join("npm").join("dsh.cmd");
        if shim.exists() {
            let mut cmd = std::process::Command::new("cmd");
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(CREATE_NO_WINDOW);
            }
            cmd.args([
                "/C",
                shim.to_string_lossy().as_ref(),
                "web",
                "--port",
                &port_str,
            ]);
            return Some(cmd);
        }

        // 3) node + bin.js（兜底）
        let bin = std::path::Path::new(&appdata)
            .join("npm")
            .join("node_modules")
            .join("@deepseek-ai")
            .join("dsh")
            .join("lib")
            .join("bin.js");
        if bin.exists() {
            let node = find_on_path("node").or_else(|| {
                let p = std::path::Path::new(r"C:\Program Files\nodejs\node.exe");
                if p.exists() {
                    Some(p.to_path_buf())
                } else {
                    None
                }
            })?;
            let mut cmd = std::process::Command::new(node);
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(CREATE_NO_WINDOW);
            }
            cmd.arg(bin).args(["web", "--port", &port_str]);
            return Some(cmd);
        }
    }
    None
}

/// 终止进程树（Windows 使用 taskkill /T）
fn kill_process_tree(child: &mut std::process::Child) {
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

// ========================================================================= //
// 健康检查
// ========================================================================= //

/// 同步健康检查（供 sync 命令使用）
fn dsh_healthy_blocking(port: u16) -> bool {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    match client.get(format!("http://127.0.0.1:{}/", port)).send() {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// 异步健康检查（供监控任务使用）
async fn dsh_healthy(port: u16) -> bool {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build();
    match client {
        Ok(c) => match c
            .get(format!("http://127.0.0.1:{}/", port))
            .send()
            .await
        {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        },
        Err(_) => false,
    }
}

// ========================================================================= //
// 启动 / 停止
// ========================================================================= //

/// 生成 rpcId（UUID 字符串；无 uuid 依赖，用 getrandom）
fn mint_rpc_id() -> String {
    let mut buf = [0u8; 16];
    if getrandom::getrandom(&mut buf).is_err() {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        return format!("{:032x}", t as u128 & 0xFFFF_FFFF_FFFF_FFFF);
    }
    buf[6] = (buf[6] & 0x0f) | 0x40; // version 4
    buf[8] = (buf[8] & 0x3f) | 0x80; // variant
    let mut s = String::with_capacity(36);
    for (i, b) in buf.iter().enumerate() {
        if i == 4 || i == 6 || i == 8 || i == 10 {
            s.push('-');
        }
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// 启动 DSH（幂等）。若检测到端口上已有 DSH 在运行，则直接接入不重复启动。
pub fn start_dsh(app: AppHandle) -> Result<DshStatus, String> {
    let state: tauri::State<crate::app_state::AppState> = app.state();
    let mut mgr = state.dsh.lock().unwrap();

    match mgr.state {
        DshState::Running | DshState::Starting => return Ok(mgr.status()),
        DshState::Stopping => return Err("DSH is stopping, please wait".into()),
        _ => {}
    }

    let port = mgr.port;

    // 同步健康预检：如果端口上已经有 DSH（用户自行启动的实例），直接接入
    if dsh_healthy_blocking(port) {
        mgr.set_state(DshState::Running, None);
        mgr.owned = false;
        let status = mgr.status();
        drop(mgr);
        dsh_log(&app, &format!("detected external DSH on port {}", port));
        let _ = emit(&app, events::DSH_STATUS, status.clone());
        let _ = emit(&app, events::DSH_NOTIFY, serde_json::json!({ "kind": "ready" }));
        refresh_tray_menu(&app);
        start_watcher(app);
        return Ok(status);
    }

    // 解析启动命令
    let Some(mut cmd) = resolve_dsh_command(port) else {
        let err = "DeepSeek Harness (dsh) not found. Please install it via: npm install -g @deepseek-ai/dsh"
            .to_string();
        mgr.set_state(DshState::Error, Some(err.clone()));
        let status = mgr.status();
        drop(mgr);
        dsh_log(&app, &format!("start failed: {}", err));
        let _ = emit(&app, events::DSH_STATUS, status.clone());
        refresh_tray_menu(&app);
        return Err(err);
    };

    match cmd.spawn() {
        Ok(child) => {
            mgr.child = Some(child);
            mgr.owned = true;
            mgr.set_state(DshState::Starting, None);
            let status = mgr.status();
            drop(mgr);

            dsh_log(&app, &format!("spawned dsh web --port {} (hidden)", port));
            let _ = emit(&app, events::DSH_STATUS, status.clone());
            refresh_tray_menu(&app);

            // 后台等待就绪
            let app2 = app.clone();
            tauri::async_runtime::spawn(async move {
                wait_until_ready(app2, port).await;
            });

            Ok(status)
        }
        Err(e) => {
            let err = format!("failed to spawn dsh: {}", e);
            mgr.set_state(DshState::Error, Some(err.clone()));
            let status = mgr.status();
            drop(mgr);
            dsh_log(&app, &format!("start failed: {}", err));
            let _ = emit(&app, events::DSH_STATUS, status.clone());
            refresh_tray_menu(&app);
            Err(err)
        }
    }
}

/// 轮询等待 DSH 就绪；成功后标记 Running 并启动事件监控。
async fn wait_until_ready(app: AppHandle, port: u16) {
    let deadline = Instant::now() + Duration::from_secs(DSH_READY_TIMEOUT_SECS);
    loop {
        if dsh_healthy(port).await {
            let status = {
                let state: tauri::State<crate::app_state::AppState> = app.state();
                let mut mgr = state.dsh.lock().unwrap();
                if mgr.state == DshState::Starting || mgr.state == DshState::Stopped {
                    mgr.set_state(DshState::Running, None);
                    mgr.owned = true;
                }
                mgr.status()
            };
            dsh_log(&app, "dsh web is ready");
            let _ = emit(&app, events::DSH_STATUS, status);
            let _ = emit(&app, events::DSH_NOTIFY, serde_json::json!({ "kind": "ready" }));
            refresh_tray_menu(&app);
            start_watcher(app);
            return;
        }

        // 子进程提前退出（端口冲突 / 启动失败）
        let child_exited = {
            let state: tauri::State<crate::app_state::AppState> = app.state();
            let mut mgr = state.dsh.lock().unwrap();
            if let Some(child) = mgr.child.as_mut() {
                matches!(child.try_wait(), Ok(Some(_)))
            } else {
                false
            }
        };
        if child_exited {
            let status = {
                let state: tauri::State<crate::app_state::AppState> = app.state();
                let mut mgr = state.dsh.lock().unwrap();
                mgr.set_state(
                    DshState::Error,
                    Some("dsh process exited during startup (port conflict?)".into()),
                );
                mgr.status()
            };
            dsh_log(&app, "dsh exited during startup");
            let _ = emit(&app, events::DSH_STATUS, status);
            refresh_tray_menu(&app);
            return;
        }

        if Instant::now() >= deadline {
            let status = {
                let state: tauri::State<crate::app_state::AppState> = app.state();
                let mut mgr = state.dsh.lock().unwrap();
                mgr.set_state(
                    DshState::Error,
                    Some(format!(
                        "DSH did not become ready within {}s",
                        DSH_READY_TIMEOUT_SECS
                    )),
                );
                mgr.status()
            };
            dsh_log(&app, "dsh startup timed out");
            let _ = emit(&app, events::DSH_STATUS, status);
            refresh_tray_menu(&app);
            return;
        }

        tokio::time::sleep(Duration::from_millis(DSH_HEALTH_POLL_MS)).await;
    }
}

/// 停止 DSH。仅当 DSH 由本程序启动时才会终止进程；
/// 用户自行启动的实例不会被杀掉，仅解除关联并关闭面板。
pub fn stop_dsh(app: AppHandle) -> Result<DshStatus, String> {
    // 1) 取出待终止的子进程（先释放锁再执行耗时 kill）
    let child_to_kill: Option<std::process::Child> = {
        let state: tauri::State<crate::app_state::AppState> = app.state();
        let mut mgr = state.dsh.lock().unwrap();
        if mgr.state == DshState::Stopped || mgr.state == DshState::Error {
            mgr.set_state(DshState::Stopped, None);
            let status = mgr.status();
            drop(mgr);
            let _ = emit(&app, events::DSH_STATUS, status.clone());
            refresh_tray_menu(&app);
            return Ok(status);
        }
        mgr.set_state(DshState::Stopping, None);
        if mgr.owned {
            mgr.child.take()
        } else {
            None
        }
    };

    // 2) 终止进程树
    if let Some(mut child) = child_to_kill {
        dsh_log(&app, "stopping dsh process tree");
        kill_process_tree(&mut child);
    }

    // 3) 标记停止并通知
    let status = {
        let state: tauri::State<crate::app_state::AppState> = app.state();
        let mut mgr = state.dsh.lock().unwrap();
        mgr.set_state(DshState::Stopped, None);
        mgr.status()
    };

    dsh_log(&app, "dsh stopped");
    let _ = emit(&app, events::DSH_STATUS, status.clone());
    let _ = emit(&app, events::DSH_NOTIFY, serde_json::json!({ "kind": "down" }));
    close_dsh_panel(&app);
    refresh_tray_menu(&app);
    Ok(status)
}

// ========================================================================= //
// 事件监控（任务完成检测）
// ========================================================================= //

/// 启动 DSH 事件监控（WebSocket 断线自动重连）。
/// 使用代数机制保证同一时刻只有一个 watcher 在运行：新 watcher 启动时，
/// 旧 watcher 会在下一个循环迭代检测到代数落后而退出。
pub fn start_watcher(app: AppHandle) {
    let seq = {
        let state: tauri::State<crate::app_state::AppState> = app.state();
        let mut mgr = state.dsh.lock().unwrap();
        mgr.watcher_seq += 1;
        mgr.watcher_seq
    };
    tauri::async_runtime::spawn(async move {
        watcher_loop(app, seq).await;
    });
}

async fn watcher_loop(app: AppHandle, seq: u64) {
    use futures_util::StreamExt;

    loop {
        // 代数检查：已有更新的 watcher 接管时，本任务退出
        let current_seq = {
            let state: tauri::State<crate::app_state::AppState> = app.state();
            let mgr = state.dsh.lock().unwrap();
            mgr.watcher_seq
        };
        if current_seq != seq {
            return;
        }

        let port = {
            let state: tauri::State<crate::app_state::AppState> = app.state();
            let mgr = state.dsh.lock().unwrap();
            mgr.port
        };
        let url = format!("ws://127.0.0.1:{}/api/events.host", port);

        match tokio_tungstenite::connect_async(&url).await {
            Ok((mut socket, _)) => {
                loop {
                    let msg = tokio::time::timeout(Duration::from_secs(75), socket.next()).await;
                    match msg {
                        Err(_) => break, // 75s 无帧视为连接僵死，重连
                        Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text)))) => {
                            handle_host_frame(&app, &text);
                        }
                        Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_)))) => break,
                        Ok(Some(Err(_))) => break,
                        Ok(Some(_)) => {}
                        Ok(None) => break,
                    }
                }
                let _ = socket.close(None).await;
            }
            Err(_) => {}
        }

        // 重连前检查宿主状态；若 DSH 已不在运行则结束 watcher
        let still_running = {
            let state: tauri::State<crate::app_state::AppState> = app.state();
            let mgr = state.dsh.lock().unwrap();
            mgr.state == DshState::Running || mgr.state == DshState::Starting
        };
        if !still_running {
            return;
        }

        // 健康检查失败 ⇒ DSH 已退出（或端口被释放），标记停止
        if !dsh_healthy(port).await {
            let status = {
                let state: tauri::State<crate::app_state::AppState> = app.state();
                let mut mgr = state.dsh.lock().unwrap();
                mgr.set_state(DshState::Stopped, None);
                mgr.status()
            };
            dsh_log(&app, "dsh server went away (health check failed)");
            let _ = emit(&app, events::DSH_STATUS, status);
            let _ = emit(&app, events::DSH_NOTIFY, serde_json::json!({ "kind": "down" }));
            close_dsh_panel(&app);
            refresh_tray_menu(&app);
            return;
        }

        tokio::time::sleep(Duration::from_millis(DSH_WS_RECONNECT_MS)).await;
    }
}

/// 处理一帧 host 事件
fn handle_host_frame(app: &AppHandle, text: &str) {
    let Ok(env) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    let Some(payload) = env.get("payload").and_then(|p| p.as_object()) else {
        return;
    };
    let Some(frame_type) = payload.get("type").and_then(|t| t.as_str()) else {
        return;
    };

    match frame_type {
        "host/session-status" => {
            let Some(session_id) = payload.get("sessionId").and_then(|v| v.as_str()) else {
                return;
            };
            let running = payload
                .get("running")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            // 在锁内完成状态更新并取出“本轮是否结束”的判定结果
            let outcome: Option<(bool, u64)> = {
                let state: tauri::State<crate::app_state::AppState> = app.state();
                let mut mgr = state.dsh.lock().unwrap();
                let session_key = session_id.to_string();

                let entry = mgr
                    .session_runs
                    .entry(session_key.clone())
                    .or_insert(SessionRun {
                        running: false,
                        errored: false,
                        since: Instant::now(),
                    });

                if running {
                    if !entry.running {
                        entry.running = true;
                        entry.errored = false;
                        entry.since = Instant::now();
                    }
                    mgr.running_sessions.insert(session_key);
                    None
                } else {
                    entry.running = false;
                    // 先取出 entry 字段，结束对 entry 的借用，再操作集合
                    let errored = entry.errored;
                    let elapsed = entry.since.elapsed().as_millis() as u64;
                    let was_in_set = mgr.running_sessions.remove(&session_key);
                    // 仅在“确实由 running 转入 idle”且“已无任何运行中的会话”时判定完成
                    if was_in_set && mgr.running_sessions.is_empty() {
                        Some((errored, elapsed))
                    } else {
                        None
                    }
                }
            };

            if let Some((errored, elapsed)) = outcome {
                if errored {
                    dsh_log(&app, &format!("task failed (session {})", session_id));
                    let _ = emit(
                        &app,
                        events::DSH_NOTIFY,
                        serde_json::json!({ "kind": "failed" }),
                    );
                } else if elapsed >= DSH_MIN_RUN_MS {
                    dsh_log(
                        &app,
                        &format!(
                            "task completed (session {}, ran {}ms)",
                            session_id, elapsed
                        ),
                    );
                    let _ = emit(
                        &app,
                        events::DSH_NOTIFY,
                        serde_json::json!({ "kind": "completed" }),
                    );
                }
            }
        }
        "host/agent-error" => {
            let Some(session_id) = payload.get("sessionId").and_then(|v| v.as_str()) else {
                return;
            };
            let state: tauri::State<crate::app_state::AppState> = app.state();
            let mut mgr = state.dsh.lock().unwrap();
            mgr.session_runs
                .entry(session_id.to_string())
                .and_modify(|e| e.errored = true)
                .or_insert(SessionRun {
                    running: true,
                    errored: true,
                    since: Instant::now(),
                });
        }
        _ => {}
    }
}

// ========================================================================= //
// DSH 交互面板窗口
// ========================================================================= //

/// 打开/收起 DSH 交互面板（停靠于桌宠旁边）
pub fn toggle_dsh_panel(app: &AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(WINDOW_LABEL_DSH) {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
            return Ok(());
        }
        position_panel_near_pet(app, &win);
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
        return Ok(());
    }

    let state: tauri::State<crate::app_state::AppState> = app.state();
    let (port, dsh_state) = {
        let mgr = state.dsh.lock().unwrap();
        (mgr.port, mgr.state)
    };
    if dsh_state != DshState::Running {
        return Err("DSH is not running".into());
    }

    let url: tauri::Url = format!("http://127.0.0.1:{}/", port)
        .parse()
        .map_err(|e| format!("invalid URL: {}", e))?;

    let win = WebviewWindowBuilder::new(app, WINDOW_LABEL_DSH, WebviewUrl::External(url))
        .title("DeepSeek Harness")
        .inner_size(DSH_PANEL_WIDTH, DSH_PANEL_HEIGHT)
        .min_inner_size(DSH_PANEL_MIN_WIDTH, DSH_PANEL_MIN_HEIGHT)
        .resizable(true)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;

    crate::apply_window_icon(app, &win);
    position_panel_near_pet(app, &win);
    let _ = win.show();
    let _ = win.set_focus();
    dsh_log(app, "dsh panel opened");
    Ok(())
}

/// 关闭 DSH 交互面板
pub fn close_dsh_panel(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(WINDOW_LABEL_DSH) {
        let _ = win.destroy();
    }
}

/// 将面板停靠到桌宠渲染窗口旁边（屏幕边缘自动翻转）
fn position_panel_near_pet(app: &AppHandle, panel: &tauri::WebviewWindow) {
    let Some(pet) = crate::get_render_window(app) else {
        let _ = panel.center();
        return;
    };
    let (Ok(pet_pos), Ok(pet_size)) = (pet.outer_position(), pet.outer_size()) else {
        let _ = panel.center();
        return;
    };

    let scale = pet.scale_factor().unwrap_or(1.0);
    let pet_x = pet_pos.x as f64 / scale;
    let pet_y = pet_pos.y as f64 / scale;
    let pet_w = pet_size.width as f64 / scale;

    let Ok(panel_size) = panel.outer_size() else {
        let _ = panel.center();
        return;
    };
    let panel_w = panel_size.width as f64 / scale;
    let panel_h = panel_size.height as f64 / scale;

    let (m_x, m_y, m_w, m_h) = match panel.current_monitor().ok().flatten() {
        Some(m) => {
            let s = m.scale_factor();
            (
                m.position().x as f64 / s,
                m.position().y as f64 / s,
                m.size().width as f64 / s,
                m.size().height as f64 / s,
            )
        }
        None => (0.0, 0.0, 1920.0, 1080.0),
    };

    const GAP: f64 = 12.0;
    // 默认放桌宠右侧，放不下则放左侧
    let mut x = pet_x + pet_w + GAP;
    if x + panel_w > m_x + m_w - GAP {
        x = pet_x - panel_w - GAP;
    }
    if x < m_x + GAP {
        x = m_x + GAP;
    }

    // 垂直方向：与桌宠顶部对齐，超出屏幕则上移
    let mut y = pet_y;
    if y + panel_h > m_y + m_h - GAP {
        y = (m_y + m_h - GAP - panel_h).max(m_y + GAP);
    }

    let _ = panel.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(x, y)));
}

/// 在默认浏览器中打开 DSH Web UI
pub fn open_dsh_in_browser(app: &AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let state: tauri::State<crate::app_state::AppState> = app.state();
    let (port, dsh_state) = {
        let mgr = state.dsh.lock().unwrap();
        (mgr.port, mgr.state)
    };
    if dsh_state != DshState::Running {
        return Err("DSH is not running".into());
    }
    let url = format!("http://127.0.0.1:{}/", port);
    app.opener()
        .open_url(&url, None::<&str>)
        .map_err(|e| format!("{}", e))
}

// ========================================================================= //
// 托盘菜单刷新
// ========================================================================= //

/// 刷新托盘菜单（DSH 状态变化时调用，保持菜单项可用性同步）
pub fn refresh_tray_menu(app: &AppHandle) {
    use crate::modules::constants::TRAY_ID_MAIN;
    if let Some(tray) = app.tray_by_id(TRAY_ID_MAIN) {
        if let Ok(menu) = crate::inner_build_tray_menu(app) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

// ========================================================================= //
// 单实例 CLI 参数分发（--dsh-start / --dsh-stop / --dsh-panel / --dsh-browser）
// ========================================================================= //

/// 处理第二次启动传入的 DSH CLI 参数。
/// 返回 true 表示至少消费了一个参数。
pub fn handle_dsh_cli_args(app: &AppHandle, args: &[String]) -> bool {
    let mut consumed = false;
    for a in args {
        match a.as_str() {
            "--dsh-start" => {
                consumed = true;
                let _ = start_dsh(app.clone());
            }
            "--dsh-stop" => {
                consumed = true;
                let _ = stop_dsh(app.clone());
            }
            "--dsh-panel" => {
                consumed = true;
                let _ = toggle_dsh_panel(app);
            }
            "--dsh-browser" => {
                consumed = true;
                let _ = open_dsh_in_browser(app);
            }
            _ => {}
        }
    }
    consumed
}

// ========================================================================= //
// 前端资源（完成提示音）
// ========================================================================= //

/// 解析随应用打包的 DSH 音频资源绝对路径（找不到返回空字符串）
pub fn resolve_dsh_asset_path(app: &AppHandle, rel: &str) -> String {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(res) = app.path().resource_dir() {
        candidates.push(res.join("static").join("dsh").join(rel));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("static").join("dsh").join(rel));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("static").join("dsh").join(rel));
    }
    for p in candidates {
        if p.is_file() {
            return p.to_string_lossy().into_owned();
        }
    }
    String::new()
}

// ========================================================================= //
// 单元测试
// ========================================================================= //

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_serializes_snake_state() {
        let s = DshStatus {
            state: DshState::Running,
            owned: true,
            url: Some("http://127.0.0.1:3080/".into()),
            port: 3080,
            last_error: None,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"state\":\"running\""));
        assert!(json.contains("\"owned\":true"));
    }

    #[test]
    fn mint_rpc_id_is_uuid_shaped() {
        let id = mint_rpc_id();
        assert_eq!(id.len(), 36);
        assert_eq!(id.as_bytes()[14], b'4'); // version
    }

    #[test]
    fn manager_defaults_stopped() {
        let m = DshManager::default();
        assert_eq!(m.state, DshState::Stopped);
        assert!(!m.owned);
        assert_eq!(m.port, DSH_DEFAULT_PORT);
    }

    #[test]
    fn dsh_state_str_names() {
        assert_eq!(DshState::Stopped.as_str(), "stopped");
        assert_eq!(DshState::Starting.as_str(), "starting");
        assert_eq!(DshState::Running.as_str(), "running");
    }

    #[test]
    fn find_on_path_finds_cmd() {
        // Windows 上 cmd.exe 一定在 System32
        let p = find_on_path("cmd");
        assert!(p.is_some());
    }

    #[test]
    fn resolve_dsh_command_returns_something_when_installed() {
        // 本机已安装 dsh（开发/自测环境）；未安装时返回 None 也不视为失败
        if let Some(mut cmd) = resolve_dsh_command(DSH_DEFAULT_PORT) {
            // 验证命令存在（不实际执行）
            assert!(!cmd.get_program().is_empty());
        }
    }
}
