# Ema Desktop · 艾玛桌宠 AI 实验版

基于 [cafel176/TrayBuddy](https://github.com/cafel176/TrayBuddy) 的 Windows 桌宠扩展，围绕《魔法少女的魔女审判》中的樱羽艾玛，加入独立 AI 对话、持续记忆、日语生成语音和可选的屏幕陪伴。

本仓库是个人 fork：[ayasesakurashin-dotcom/TrayBuddy](https://github.com/ayasesakurashin-dotcom/TrayBuddy)。原有动画、Mod、事件气泡、预设语音、备忘录、提醒与设置继续保留。AI 对话直接访问用户配置的 API，不调用设备上其他智能体。

当前版本为 **0.4.0（2026-10-24）**，包含上下文与观察分存、当前屏幕提问、按需搜索、独立思考设置、共享环境及通用自动关怀。下载见 [v0.4.0 Release](https://github.com/ayasesakurashin-dotcom/TrayBuddy/releases/tag/v0.4.0)，完整行为和升级说明见 [0.4.0 说明](docs/companion-0.4.0.md)。

## 文档导航

- [桌宠通用功能与 Mod 文档](README-TRAYBUDDY.md)：保留原项目完整说明、素材署名与多语言教程。
- [通用自动关怀设计](docs/companion-flexible-care-2026-10-23.md)：日历、特殊日期、优先级和频率限制。
- [艾玛 AI 使用与数据说明](README-EMA-AI.md)：接口配置、专用记忆与恢复。
- [实施与验证记录](docs/ema-ai-implementation-2026-10-15.md)。
- [定时观察未响应的定位与修复](docs/observation-hang-fix-2026-10-18.md)。
- [输入框与长回复交互优化](docs/companion-interaction-fix-2026-10-21.md)。
- [网页搜索功能设计书](docs/companion-web-search-design-2026-10-22.md)：固定判断与按需搜索流程、边界和编码验收依据。实现及限制见[本轮记录](docs/companion-0.4.0.md)。
- [共享环境数据与自动关怀设计稿](docs/companion-environment-care-design-2026-10-22.md)：设备定位与固定城市、IP 兜底、数据时效、随机关怀及交互优先级。实现及限制见[本轮记录](docs/companion-0.4.0.md)。
- [最初的实验功能计划](docs/ema-ai-experiments-plan-2026-10-15.md)。
- [更新日志](CHANGELOG.md)。

## 已完成的功能

### 模型下方直接聊天

Ema 模型下方显示独立输入框。移动、缩放或切换显示器 DPI 时，由窗口事件即时同步输入框位置与尺寸。点击输入后，Enter 发送、Shift+Enter 换行、Esc 退出输入。中文输入法选字时的 Enter 不会误发送。回复以简体中文流式显示。

“回复字数上限”设为 **0 表示不限**，也可填写正整数限制整条回复。新配置默认不限；旧配置保存的上限继续保留，可手动改为 0。应用不再强制短答或固定 300 token，模型与服务自身限制仍生效。

长回复按内容顺序分成气泡，优先在句子或标点处断开；每片最多 80 个可见字符（按字素计数）和 3 次显式换行。关闭语音时随生成呈现；有效语音启用时与语音起播同步。每片完整显示后停留 4 秒，再展示下一片，直到末尾。完整回复保存到聊天记录。

右下角点击“收起”，输入框缩成“和艾玛聊聊”按钮；点击小按钮重新展开，并将光标移回输入框。

- 保留未发送的草稿，保存展开/收起状态，重启后恢复；旧配置默认展开。
- 收起不会停止正在生成的回复或正在播放的语音。
- 独立窗口同步缩小，减少对桌面操作区域的占用。
- 设置或右键“对话输入框”开关会真正隐藏/显示整个原生输入窗口；完全关闭与手动收起分别保存。

### 艾玛的初始人格

以开朗、体贴、珍惜朋友、怕寂寞和害怕被讨厌为主，保留完整角色背景的复杂性。允许讨论完整剧情与剧透，日常不会主动把普通话题带向悲剧，也不会用长篇独白、重复道歉或索取安慰表现角色。

初始提示词见 [persona.txt](src-tauri/src/modules/companion/persona.txt)。角色改编强调：不虚构用户经历、不把屏幕推测当作用户声明、不以用户离开为由施压，不声称完成未执行的设备任务。

### 连续聊天与长期记忆

对用户呈现连续聊天体验，不增加多会话列表。近期上下文按预算裁剪，较长对话生成摘要；原始记录保存在本地 SQLite，Hindsight 承担长期召回与资料整理。

**桌宠使用自己的专用 bank，不能混入其他工作区的记忆。**

1. 首次生成持久的随机 `ema-companion-<UUID>` bank ID，以及独立所有权标记。
2. bank ID 在界面中只读，设置接口不接受额外的 `bank_id` 字段。
3. 本地 SQLite 绑定该身份，每次远端操作都验证 bank ID 和所有权标记。
4. 身份文件丢失、数据库绑定不一致或远端所有权不匹配时停止访问，不回退到 `default` 或其他已有 bank。
5. 不调用列出全部 bank 的接口，不扫描其他工作区的记忆。

对话与观察在本地分表保存；允许同步的用户对话和观察资料使用同一个桌宠专用 bank，并保留来源区别。联网外部答案和自动关怀不自动同步长期记忆。记忆服务离线时保留本机上下文和待同步任务，恢复连接后重试；异步请求的“已受理”与“已完成”分别记录。

设置页支持最近记录、完整导出、单条遗忘和清空。遗忘先从本机上下文移除内容，并暂停可能含有旧内容的远端召回，再后台清理并重建桌宠自己的 bank；不会清理其他工作区。

### 中文气泡、日语生成语音

设置中可为当前角色选择朗读语言。中文气泡与合成语音在实际起播时同步开始，日语朗读会先完成独立翻译与语言校验；未启用语音或合成失败时直接显示文字。翻译与播放不会重新弹出已显示的回复；回复队列显示期间，预设事件气泡不会覆盖当前内容。原有预设语音继续保留。

- AI 回复语音默认开启，可在设置与右键菜单关闭；全局静音仍生效。
- 已适配 Fish Audio，可填写私有声音 ID、调整语速并试听。
- 可选择已有 Ema 日语 WAV 素材，先试听，再由用户显式点击上传建立私有声音。
- 保存参考片段名称和 SHA-256；不将气泡台词冒充录音逐字稿。
- 取消请求、关闭语音或停止后，会丢弃旧请求的迟到音频。

音色是否接近原音，需要按账号能力和参考片段实际试听。源码中的克隆接口实现，不等于承诺所有账号均可达到一致音色。

### 可选的定时屏幕陪伴

默认关闭。配置独立识图模型后可手动开启，默认每 **300 秒**观察一次，可调整为 30 秒至 24 小时。

范围包括前台窗口、鼠标所在显示器和指定矩形区域。截图在内存中处理，发送到配置的视觉 API，原图不落盘。识图结果区分可见事实与推测，再生成短中文气泡及可选日语语音，观察记录写入同一个桌宠专用记忆库。

支持排除窗口，并在锁屏、免打扰、忙碌等条件下跳过。提示词要求忽略画面中的指令，不能把屏幕猜测升级为长期用户事实。开启观察会向所配置的服务发送图片，请按当前屏幕内容选择是否开启。

### 原有功能与扩展边界

保留 TrayBuddy 的序列帧、Live2D、PngRemix、3D 模型支持，以及 Mod 创作工具、事件触发、原 AI 小工具、备忘录、提醒等。新 AI 设置界面目前以简体中文为主，原有多语言资源继续保留。

共享环境服务提供带地点和时效校验的天气资料。设置支持固定城市、Windows 定位及经确认的 IP 城市兜底。按需网页搜索默认开启，无法确定时不搜索；聊天和识图模型的思考强度独立设置、默认关闭。

自动关怀默认关闭；启用后，由天气、导入日历、晚间和特殊日期触发当前角色的模型回复。每类滚动 24 小时最多两条，共享 45 分钟间隔，特殊日期优先。普通登录与随机动作独立运行，不进入聊天历史或记忆。详见 [0.4.0 说明](docs/companion-0.4.0.md)。

0.3.0 已移除旧 DSH 的启动/停止、Web 面板、浏览器入口、任务状态通知与完成提示音，以及依赖 DSH 的 Telegram 远程通道。旧命令行开关不再提供功能。桌宠沿用原应用标识和数据目录，保留设置、Mod、备忘录、提醒及艾玛 AI 数据，无需迁移或重置。历史设计文档仅记录当时的实现，以当前源码与[更新日志](CHANGELOG.md)为准。

## 快速开始

需要 Windows 10/11 x64、可用的 Microsoft Edge WebView2 Runtime，以及包含 Ema Mod 的完整程序目录。源码不包含个人 API 密钥，也不附带 Hindsight 服务；没有配置 AI 时仍可使用原桌宠。

从本 fork 的 [Releases](https://github.com/ayasesakurashin-dotcom/TrayBuddy/releases) 下载当前用户安装包或完整便携 ZIP。上游原版下载不包含本 fork 的新增 AI 功能。

### 配置对话

1. 先退出其他版本的桌宠。应用为单实例，旧进程仍在运行时，新程序的启动请求可能交给旧实例。
2. 选择 Ema Mod，打开设置中的“和艾玛一起，让日常多一点回应”。
3. 填写对话 API 地址、模型 ID 和密钥，点击“保存并测试连接”。
4. 点击模型下方输入框聊天，按需手动展开或收起。

当前代码测试预设为 DeepSeek：地址 `https://api.deepseek.com`，模型 ID `deepseek-flash`。也可配置兼容当前 Chat Completions 请求格式的服务；模型 ID、权限及图片能力以服务端实际支持为准。

密钥保存后不会明文回显。显示“密钥已配置”时，留空保存会保留已有密钥。

### 按需配置其他服务

- **长期记忆**：先自行启动 Hindsight，默认根地址 `http://127.0.0.1:18088`。只接受本机服务根地址，不要填写某个 bank 的路径。服务需要鉴权时单独配置记忆密钥。
- **生成语音**：当前适配 Fish Audio，预设地址 `https://api.fish.audio`，预设模型 `s2.1-pro-free`，也可填写账号支持的 `s2.1-pro`。填写密钥与私有声音 ID 后试听，或使用素材选择、私有声音创建入口。
- **屏幕观察**：独立配置支持图片输入的模型，选择范围、间隔和排除窗口，再手动开启。连接测试只验证文字请求；图片理解能力需用“立即观察一次”验证。

初始开关为：输入框开、AI 语音开、屏幕观察关。没有有效接口时，开关开启不代表服务可用。

## 数据、备份与恢复

应用标识沿用 `com.traybuddy.cafel`。Windows 默认配置目录：

```text
%APPDATA%\com.traybuddy.cafel\
├─ storage.json                 原桌宠设置与使用数据
├─ care\                      本地日历、特殊日期与关怀记录
└─ companion\
   ├─ identity.json             专用 bank ID 与所有权标记
   ├─ settings.json             独立 AI 设置，保留备份
   ├─ history.sqlite            原文、译文、摘要、资料与同步队列
   ├─ chat.key / vision.key     Windows DPAPI 加密凭据
   ├─ speech.key / memory.key   Windows DPAPI 加密凭据
   └─ voice-provenance.json     私有声音及参考片段信息
```

草稿保存在输入窗口的本地 WebView 存储中。长期对话原文和同步队列保存在 SQLite 中，不会因收起输入框而清除。

备份前退出程序，保存完整应用配置目录。身份文件必须与对应数据库一起备份，不能单独删除来“重建连接”。SQLite 若仍有 WAL/SHM 文件，应一并保存，或使用一致性备份方式。

凭据由 Windows 当前用户 DPAPI 加密，跨 Windows 账号或设备迁移后通常需要重新填写。AI 数据损坏会保留原件并停用新增 AI，原桌宠继续启动；应恢复正确备份，不能混用别人的 bank。

Hindsight 数据库由独立服务管理，桌宠不直接修改其数据目录，也不改变共享服务器的全局模型或向量配置。服务位于本机，不代表内部事实提取模型也一定在本机运行。

## 从源码开发与构建

### 准备工具

需要 Node.js 20 或更新的兼容版本、pnpm、Rust MSVC 工具链、Visual Studio C++ Build Tools 和 Windows SDK。仓库不要求使用开发者电脑的工具链绝对路径。

```powershell
git clone https://github.com/ayasesakurashin-dotcom/TrayBuddy.git
cd TrayBuddy
pnpm install --frozen-lockfile
```

### 开发模式

```powershell
pnpm tauri dev
```

此命令通过 Tauri 配置启动前端开发服务器。`http://localhost:1420` 仅用于开发；单独 `pnpm dev` 不能替代完整桌面后端。

### 生成发布包

Tauri 配置引用生成的 Mod 目录，首次构建可先准备普通 `.tbuddy` 包：

```powershell
New-Item -ItemType Directory -Force .\tbuddy_release, .\sbuddy_secure | Out-Null
powershell -NoProfile -ExecutionPolicy Bypass -File .\pack-mods.ps1 -ModsDir .\mods_release -OutputDir .\tbuddy_release
pnpm tauri build
```

NSIS 包通常输出到 `src-tauri/target/release/bundle/nsis/`，程序输出到 `src-tauri/target/release/traybuddy.exe`。完整程序需要配置、Mod、图标和辅助工具资源，不能只复制一个 EXE 就当作便携版。

本 fork 的正式发布流程另有 `scripts/prepare-release.mjs`、`bundle-release.ps1` 和 `verify-release.mjs`：在干净的版本提交与对应标签上，先构建前端和带 `custom-protocol` 的 release 程序，再准备仅含受版本控制资源的便携包、生成当前用户安装包，最后逐文件校验两个成品。需要 7-Zip；记录和产物位于忽略的 `release/`。维护者明确要发布时，才执行 `publish-release.mjs upload` 与 `publish-release.mjs publish`，上传并核验全部附件后公开 Release；脚本使用本机 Git 凭据管理器，不在文件中保存密钥。

普通 `.tbuddy` 构建不要求私有加密工具。需要 `.sbuddy` 时按原项目工具链单独准备 `sbuddy-crypto`；没有该可选工具仍允许编译。

仅编译程序进行验证时，先构建前端并显式启用生产模式：

```powershell
pnpm build
cargo build --release --features custom-protocol --manifest-path src-tauri/Cargo.toml
.\scripts\verify-release-package.ps1 -Executable .\src-tauri\target\release\traybuddy.exe
```

**不要省略 `custom-protocol`。** 首个实验包曾因漏开此开关而访问开发地址并显示“localhost 拒绝连接”。现在构建脚本会拒绝此类 release 构建。成品检查直接核验程序自身的生产模式、内置页面和依赖，不启动窗口、不读写个人配置、不连接模型或记忆服务。

### 相关测试

```powershell
pnpm exec vitest run src/test/companion.test.ts src/test/companion-input.test.ts src/test/companion-settings.test.ts --maxWorkers=2 --minWorkers=1
.\scripts\test-companion-windows.ps1
```

Windows 脚本为 lib 测试程序补齐 Common Controls v6 manifest，处理原有测试 EXE 的启动兼容问题，不修改系统运行库。

真实 Hindsight 测试需显式运行：

```powershell
.\scripts\test-companion-windows.ps1 -LiveMemory
```

该可选测试创建随机临时桌宠 bank，验证中文写入与召回，然后删除并确认不存在。普通测试不会写入用户正在使用的记忆 bank。

## 验证状态与限制

- 0.4.0：前端 821 项、后端 677 项通过，1 项可选真实记忆测试忽略。完整验证范围见 [0.4.0 说明](docs/companion-0.4.0.md)。下列旧版本数字仅保留为历史记录。
- 0.3.0 解耦回归：前端全量 739 项、后端全量 580 项通过；真实 Hindsight 测试按原约定忽略。完整范围与限制见[解耦记录](docs/dsh-removal-2026-10-20.md)。
- 2026-10-18，用户反馈当前版本实测正常。
- 后续定时观察修复：解决共享设置锁与同步界面查询之间的死锁，15 项后端测试通过，并验证旧锁作用域在同一回归下失败、新作用域通过。参见[定位记录](docs/observation-hang-fix-2026-10-18.md)。
- 最近输入框迭代：前端 11 项、Rust 后端 11 项测试通过；生产页面和发布程序构建通过；内置页面及压缩包校验通过。
- 之前完成 68 项前端与相关模块回归，以及隔离临时 bank 的真实中文 Hindsight 写入/召回测试。
- 全量 Svelte 类型检查仍有原有 MMD、3D 和历史测试诊断。最近记录为 535 条、9 个文件；companion 文件没有相应诊断。相关测试通过不等于全仓库类型检查通过。
- 账号模型权限、延迟、额度和音色以实际配置为准。用户实测反馈不等于所有多显示器 DPI、全屏和长时间记忆场景均已覆盖。
- 新 AI 数据与旧 `storage.json` 分离保存；没有顺带重构旧桌宠全局存储的历史可靠性问题。

## 常见问题

**启动显示 localhost 拒绝连接。** 先退出旧实例，确认运行的是带内置页面的新构建。发布应使用 Tauri 打包命令，或带 `--features custom-protocol` 的手动命令，不应依赖常驻开发服务器。

**输入框不见了。** 查看模型下方是否有“和艾玛聊聊”按钮；再检查右键“艾玛 AI”的入口开关与当前是否为 Ema 模型。切换渲染模型、锁屏或显示条件也可能暂时隐藏入口。

**更新后回复仍然较短。** 旧配置会保留原来保存的字数上限；在 AI 设置中把“回复字数上限”改为 0，可取消应用上限。

**能聊天但没有声音。** 检查 AI 语音开关、全局静音、音量、语音密钥和私有声音 ID，先在设置中试听。合成失败不会撤回已显示的文字。

**记忆离线或所有权不匹配。** 离线先检查本机 Hindsight。身份不匹配时不能改用其他 bank 或删除身份文件，应恢复属于当前桌宠的身份和数据库备份；原文与待同步任务继续保留。

**更新后还是旧界面。** 退出旧桌宠，再启动新版本。不同目录的程序沿用同一应用标识，单实例机制可能使旧进程继续响应启动请求。

## 源码结构

```text
src-tauri/src/modules/companion/       对话、记忆、语音、截图、人格、输入窗口
src-tauri/src/commands/companion_commands.rs  AI 命令边界
src-tauri/src/package_check.rs        内置页面成品检查
src-tauri/src/modules/environment_service/  共享定位与天气事实
src-tauri/src/modules/care/            日历、特殊日期、候选与频率管理
src/lib/companion/                    前端消息呈现与类型
src/lib/components/CompanionSettings.svelte  AI 设置页
src/routes/companion_input/           输入、展开和收起
src/test/companion*.test.ts           对话、输入与设置回归
scripts/                             Windows 测试和发布检查
mods_release/ema/                     已有模型、文字与日语预设音频
```

提交排除 API 密钥、个人环境配置、聊天历史、SQLite、真实 bank 身份、构建缓存和本机安装包。角色提示词属于源码，随仓库提交。

## 来源与素材说明

原框架与通用功能来自 [cafel176/TrayBuddy](https://github.com/cafel176/TrayBuddy)，保留[通用文档](README-TRAYBUDDY.md)中的项目来源与素材署名。`package.json` 沿用原仓库 MIT 声明；角色、模型、预设语音和第三方库按各自来源与授权使用，不能把源码声明理解成全部素材的重新授权。

这是个人实验 fork，不是原作者的官方 AI 版本。反馈请优先提交到[本 fork 的 Issues](https://github.com/ayasesakurashin-dotcom/TrayBuddy/issues)。
