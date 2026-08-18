# TrayBuddy × DeepSeek Harness 联动版（DSH 桌宠交互界面）

> 在保留 [TrayBuddy](https://github.com/cafel176/TrayBuddy) 全部桌宠功能的基础上，
> 为桌宠接入 **DeepSeek Harness (DSH)**：右键桌宠即可启动 DSH、在桌宠旁边直接与 DSH 对话，
> 任务完成时樱羽艾玛会播放提示音并弹出气泡。

本仓库是独立源码仓库，包含完整可构建源码与打包产物（NSIS 安装包）。

---

## ✨ DSH 联动功能

| 功能 | 说明 |
| --- | --- |
| **手动启动 DSH** | 右键桌宠（或系统托盘图标）→ `DeepSeek Harness → 启动 DSH`，在**隐藏终端**中运行 `dsh web` 启动服务器；**不自动打开浏览器**。检测到已有 DSH 在运行时直接接入，不重复启动 |
| **手动停止 DSH** | 菜单 `停止 DSH` 终止由桌宠启动的进程树；用户自行启动的实例不会被误杀（仅解除关联） |
| **DSH 交互面板** | 右键桌宠 → `打开交互面板`，在**桌宠旁边**弹出 DSH 官方 Web UI 窗口（完整功能：会话、工具、模型选择、子代理、目标、设置等），窗口大小适中、可缩放、屏幕边缘自动翻转停靠 |
| **任务完成提醒** | 桌宠通过 DSH 的 `host/session-status` 事件流（WebSocket `/api/events.host`）实时感知 Agent 状态：所有 Agent 由运行转入空闲 ⇒ 判定任务完成，**播放樱羽艾玛语音**（`static/dsh/ema_amazing.ogg`，来自 TrayBuddy ema mod 的 `amazing.ogg`）并弹出「任务完成啦~」气泡；出错时弹出失败气泡 |
| **兼容用户数据** | 沿用原 TrayBuddy 的应用标识（`com.traybuddy.cafel`）与存储路径，安装新版后原设置、Mod、备忘录、提醒等用户数据全部保留 |
| **不改变自启动** | 桌宠的开机自启功能保持原样未改动；DSH **不会**随开机自启，始终由用户手动启动 |

### 菜单入口

- **桌宠右键**：在桌宠身上右键 → `DeepSeek Harness` 子菜单（与托盘菜单相同）。
- **托盘图标**：Windows 右下角托盘图标右键 → 同一子菜单。

菜单项按状态自动启用/禁用：

| 菜单项 | 何时可用 |
| --- | --- |
| 启动 DSH | DSH 未运行时 |
| 停止 DSH | DSH 启动中 / 运行中 |
| 打开交互面板 | DSH 运行中 |
| 在浏览器中打开 | DSH 运行中 |

### 命令行（可选，便于自动化/快捷方式）

桌面宠支持单实例参数转发，可创建快捷方式：

```bat
traybuddy.exe --dsh-start     :: 启动 DSH
traybuddy.exe --dsh-stop      :: 停止 DSH
traybuddy.exe --dsh-panel     :: 打开/收起交互面板
traybuddy.exe --dsh-browser   :: 在浏览器打开
```

---

## 🔌 与 DSH 的通信方式（实现原理）

参考 [dshpet (dsh-kun-like-pet)](https://github.com/liyupi/dsh-kun-like-pet) 的状态推导思路，
但 TrayBuddy 作为**独立桌面应用**直接与 DSH 服务器通信，无需安装任何 DSH 插件：

1. **启动**：以 `CREATE_NO_WINDOW` 隐藏终端运行 `cmd /C dsh web --port 3080`
   （依次尝试 PATH 中的 `dsh`、`%APPDATA%\npm\dsh.cmd`、`node …\@deepseek-ai\dsh\lib\bin.js`）。
2. **就绪检测**：轮询 `GET http://127.0.0.1:3080/`，200 即就绪（首次启动约 15~25 秒）。
3. **交互面板**：Tauri `WebviewWindow` 加载 `http://127.0.0.1:3080/`（DSH 官方 Web UI 本体，功能 100% 一致）。
4. **任务完成检测**：连接 WebSocket `ws://127.0.0.1:3080/api/events.host`，
   解析 `host/session-status`（running 标志）与 `host/agent-error` 帧：
   - 会话进入 running 时记录开始时间；
   - 所有会话全部回到 idle 且期间无 agent-error ⇒ 发出 `completed` 通知；
   - 期间出现 agent-error ⇒ 发出 `failed` 通知。
   断线自动重连；DSH 意外退出时自动感知并同步菜单状态。
5. **提醒呈现**：后端向桌宠窗口发 `dsh-notify` 事件 → 桌宠弹出气泡，
   `completed` 时通过打包资源路径 + `convertFileSrc` 播放 `ema_amazing.ogg`（遵循桌宠音量/静音设置）。

诊断日志写入 `%APPDATA%\com.traybuddy.cafel\dsh.log`。

---

## 📦 构建

环境要求与 [TrayBuddy](https://github.com/cafel176/TrayBuddy) 相同（Node 18+ / pnpm 8+ / Rust 1.75+ / VS2022 Build Tools）。

```bash
pnpm install

# 开发运行
pnpm tauri dev

# 打包安装包（NSIS）
pnpm tauri build
# 产物：src-tauri/target/release/bundle/nsis/*.exe
# 直接运行版：src-tauri/target/release/traybuddy.exe
```

发布版 mod 打包流程见 `release.bat`（与上游一致）。

---

## ⚠️ 说明

- 任务完成提示音 `static/dsh/ema_amazing.ogg` 复制自 TrayBuddy 的 `ema`（樱羽艾玛）mod
  （`mods_release/ema/audio/jp/speech/amazing.ogg`），版权归原作者所有，仅随本程序使用。
- 需预先安装 DeepSeek Harness：`npm install -g @deepseek-ai/dsh`（https://github.com/deepseek-ai/deepseek-harness）。
- DSH 端口默认 `3080`（与 DSH web profile 默认一致）。
