# 艾玛 AI 实验版

本次新增桌宠自带的短句对话、连续记忆、日语合成语音和定时屏幕观察。DSH 和原有页面识别工具继续独立运行；没有接入其他本机智能体。

## 开始使用

1. 使用 Ema 模型，在桌宠设置中找到“和艾玛一起，让日常多一点回应”。
2. 配置对话 API。DeepSeek 官方地址为 `https://api.deepseek.com`，V4.1 Flash 的请求模型名为 `deepseek-flash`。填写密钥后点击“保存并测试连接”。
3. 点击模型下方的输入框聊天。右下角“收起”可缩成“和艾玛聊聊”小按钮，点击小按钮重新展开；保留草稿并记住展开/收起状态，重启后恢复，不会中断回复或语音。Enter 发送，Shift+Enter 换行，Esc 退出输入。日常回复以 1—3 句为主，默认硬上限 120 字，设置允许 40—180 字。
4. 需要声音时，配置 Fish Audio 地址 `https://api.fish.audio`、模型 `s2.1-pro-free`（或账号可用的 `s2.1-pro`）和密钥。可以填已有私有声音 ID，也可以选择 Ema 的原有 WAV 素材，先试听，再上传建立私有声音。
5. 需要屏幕陪伴时，独立配置支持图片输入的模型，再手动开启屏幕观察。默认每 300 秒观察前台窗口，可切换到鼠标所在显示器或指定区域。截图会发送至所配置的视觉 API，原图不落盘。

输入框和 AI 语音默认开启；未配置有效接口时保持待配置状态。屏幕观察默认关闭。右键“艾玛 AI”子菜单可快速切换三个开关、停止回复或手动观察一次。

## 记忆严格隔离

- Hindsight 默认连接 `http://127.0.0.1:18088`，只允许填写本机服务根地址。
- 首次使用时在应用数据目录的 `companion/identity.json` 中生成随机、持久的专用 bank ID：`ema-companion-<UUID>`，同时生成独立所有权标记。
- bank ID 在界面中只读，保存设置的接口不接受 `bank_id` 字段。模型输出、当前工作目录和 DSH 会话不能选择 bank。
- 本地 SQLite 记录绑定该 bank ID；身份文件丢失、数据库身份不一致或远端所有权标记不匹配时停止接入，不回退到 `default`、已有编程助手或其他工作区 bank。
- 每次记忆检索、写入、资料反思或删除前验证远端所有权。不会调用列出全部 bank 的接口。
- 手动对话和屏幕观察写入同一专用 bank，来源保持区分。长期记忆断线时使用本地上下文，原文和待同步任务保留在 SQLite。
- 每轮具有独立文档 ID、稳定异步操作 ID。请求已受理和记忆已处理完成分别记录，避免将异步受理误判成同步成功。
- 忘记一条或清空记忆后，暂停旧远端检索，清理并从允许保留的本地记录重建桌宠自己的 bank，覆盖派生资料。不会删除其他 bank。

Hindsight 的数据库仍由现有服务管理：本机已确认目录为 `%USERPROFILE%\.pg0\instances\hindsight\data`。桌宠不直接操作这些文件，不改变服务器的全局 embedding、模型配置或其他 bank。Hindsight 虽然部署在本机，其配置的事实抽取模型仍可能是云端 API。

## 数据与恢复

Ema AI 的配置和记录位于 Tauri 应用配置目录中的 `companion` 子目录：

- `identity.json`：专用记忆身份，务必与记录一起备份，不要手工修改 ID。
- `settings.json`：独立 AI 设置，原子保存，并保留 `.bak`。
- `history.sqlite` 及其 SQLite 运行文件：完整对话、译文、摘要、用户资料和可靠同步队列。
- `chat.key`、`vision.key`、`speech.key`、`memory.key`：Windows 当前用户 DPAPI 加密凭据，不是明文密钥。
- `voice-provenance.json`：私有声音 ID、参考片段名称及 SHA-256。

设置中可以查看最近记录、导出全部记录、忘记某一条或清空艾玛记忆。复制正在写入的 SQLite 数据库时应使用一致性备份方式，或先退出应用并保留相关 WAL 文件。跨 Windows 用户移动加密凭据后需要重新填写密钥。

新 AI 数据损坏时会停用 AI 并保留原文件，原桌宠继续启动。恢复对应备份后重启；不要删除身份文件来尝试“自动修复”已有记忆。

## 能力与边界

人格以开朗、体贴、怕寂寞和害怕被讨厌为主，允许完整剧情但区分路线。初始提示词位于 `src-tauri/src/modules/companion/persona.txt`。

中文回复先显示，随后忠实翻成日语再合成。关闭语音、静音或停止请求会取消/丢弃旧播放结果。DSH 提醒到来时新 AI 语音主动让位，DSH 后端保持原架构。克隆音色必须在提供密钥后试听确认，接口能力不能替代实际音色验收。

仅预留了只读上下文接口：当前系统时间，以及桌宠原来就有的天气缓存。没有注册系统操作工具，没有命令执行或智能体调度循环，不能声称完成了实际设备任务。

## 开发验证

发布构建必须启用 Tauri 的 `custom-protocol`，才能将页面嵌入程序并脱离开发服务器启动。使用 `pnpm tauri build`；手动编译则先构建前端，再执行：

```powershell
pnpm build
cargo build --release --features custom-protocol --manifest-path src-tauri/Cargo.toml
.\scripts\verify-release-package.ps1 -Executable .\src-tauri\target\release\traybuddy.exe
```

漏开该功能的 release 构建会直接报错。成品检查直接读取该程序内置的页面和依赖，不创建窗口，不读写用户配置，也不连接 DSH、Hindsight 或模型 API。

普通 Rust 单元测试在本项目中可能因缺少 Common Controls v6 manifest 而无法启动。`scripts/test-companion-windows.ps1` 会编译后为生成的测试程序补齐声明，再执行桌宠测试，不更改系统运行库。

```powershell
.\scripts\test-companion-windows.ps1
.\scripts\test-companion-windows.ps1 -LiveMemory
```

`-LiveMemory` 为显式可选测试：在本机 Hindsight 创建随机临时桌宠 bank，验证真实中文写入/召回，最后删除并确认不存在。普通测试不会写入现有 Hindsight。

前端新增测试覆盖中文输入法组合事件、停止后的迟到语音、DSH 提醒让位及生成链接的纯文本呈现。前端生产构建可通过。仓库原有 MMD 库及若干历史测试文件的类型诊断与本次新增文件分开记录，不将其冒充为全部通过。
