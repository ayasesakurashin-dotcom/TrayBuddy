# 0.3.0：移除旧 DSH 集成

DSH 已迁移到独立桌面端，旧 Web 服务已从本机删除。本版本从桌宠中移除旧集成，不增加对新桌面端的依赖。

## 移除范围

- DSH 进程启动、停止、健康检查、WebSocket 会话监控、Web 面板及浏览器入口。
- 依赖 DSH 的 Telegram 远程通道，包括固定启动脚本路径、进程管理和健康监控。
- 两组托盘/右键菜单、IPC 命令、全局管理器、事件常量及前端通知监听。
- 旧 `--dsh-start`、`--dsh-stop`、`--dsh-panel`、`--dsh-browser` 参数的处理；旧快捷方式参数不会再启动服务。
- 艾玛语音对 `dsh-notify` 的让位逻辑，以及三语 DSH 文案和 `static/dsh/evening.wav` 专用副本。
- `tokio-tungstenite`、`futures-util` 直接依赖；Cargo 自动清理不再需要的 WebSocket 依赖。其他 HTTP 功能仍间接使用 `futures-util`。
- 已失效的 `README-DSH.md`；历史实施文档保留，并标明适用旧版本。

## 保留与兼容

艾玛独立对话、连续上下文、SQLite、Hindsight 专用 bank、语音合成、屏幕观察和输入框保持原有实现。取消、静音、免打扰或隐藏后仍会停止播放并丢弃迟到语音。

序列帧、Live2D、PngRemix、3D、Mod 管理与编辑工具、原 AI 工具、预设语音、天气与媒体事件、备忘录、提醒、开机自启以及单实例 Mod 导入继续保留。Ema Mod 的原始 `evening.wav` 等素材没有删除。

应用标识仍为 `com.traybuddy.cafel`，沿用原配置目录、`storage.json` 和 `companion/` 数据格式。没有用户数据迁移、清空或重建操作，不访问其他智能体的数据、插件或记忆 bank。旧 `dsh.log` 如已存在可留作历史文件，新版不再写入。

`package.json`、Cargo 清单及锁文件、Tauri 配置同步升级为 `0.3.0`。

## 验证

- 前端全量：27 个测试文件，739 项通过，包含动画、页面、Mod、音频、备忘录、提醒、艾玛设置与输入等回归。
- 后端全量：580 项通过，0 项失败；1 项真实 Hindsight 测试按原约定忽略。包含记忆所有权、加密凭据、设置原子保存、输入偏好、定时观察锁顺序、HTTP 模拟服务及原桌宠模块。
- 前端生产构建通过；生成目录中不存在旧 DSH 提示音。
- Windows 生产程序构建通过，文件版本为 `0.3.0`。便携包成品检查通过：生产模式启用，80 项内置资源，动画、输入框与设置三个页面及其 13 项引用依赖完整。
- `src/`、`src-tauri/src/`、运行时三语文案与资源中无 DSH/Telegram 集成引用。
- 全量 Svelte 检查仍报告既有 535 条错误，涉及 9 个文件，与上一版本记录一致；本轮修改的前端文件没有诊断。该检查未通过，不将生产构建或测试通过等同于全仓类型检查通过。

初次后端测试在沙箱中遇到临时文件替换与本机 HTTP 模拟连接限制；按标准 Rust 包目录，在常规权限下重跑全部通过。真实 Hindsight、付费模型/语音接口和真实桌面交互没有重新调用或验收。

构建命令沿用项目说明：先构建前端，再执行 `cargo build --release --features custom-protocol --offline --locked --manifest-path src-tauri/Cargo.toml`，随后使用 `scripts/verify-release-package.ps1` 检查程序内置页面。便携包须携带全部资源目录；退出旧实例后再运行新版本，以免单实例机制仍交由旧程序响应。
