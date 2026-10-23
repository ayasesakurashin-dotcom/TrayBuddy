# 艾玛桌宠对话上下文与观察存储优化设计书

版本：v2.0。日期：2026-10-22（Asia/Shanghai）。适用基线：Ema Desktop 0.3.1，源码 `e1b1d22`。

状态：后续编码依据，尚未实现。本稿替代本文件的长上下文初稿，补齐观察分存、引用、迁移、接口与验收。“必须”为交付要求，“初始参数”为需经实测调整的建议值。

配套设计：[带上当前屏幕功能设计书](companion-screen-question-design-2026-10-22.md)。两稿共享契约 **CTX-OBS-1**。本稿负责数据、引用、上下文、压缩、遗忘与长期同步；配套稿负责界面、目标、截图、识图与请求执行。共同接口以本稿第 4 节为准；截图与界面行为以配套稿为准。

## 1. 目标与范围

保持一个持续的桌宠对话：重启、收起输入框、切换模型均不新建 session，不增加会话列表。提高连续追问的可靠性，避免后台观察挤走对话，并在历史增长时有界压缩。

区分三类数据：

- **对话**：真实用户文字、艾玛最终回复、结束状态；艾玛主动评论屏幕也属于实际发言。
- **观察**：截图提取的文字事实、可见文字、时间、范围、来源及不确定性，不伪装成用户声明。
- **派生上下文**：滚动摘要、当前观察参考集、token 缓存，均可从允许保留的原文重建。

对话和观察在同一个 `history.sqlite` 中用不同表保存，以稳定 ID 关联；Hindsight 继续使用原专用 bank。

首版不引入本地 RAG：不增加全文/向量索引、embedding、语义检索、重排或检索路由模型。按已有 ID、顺序、时间范围读取属于确定性取回；模糊说法不足以定位旧记录时允许澄清，不承诺任意历史语义搜索。

保留 4,000 字符输入限制、人格、流式气泡、日语语音、静音/免打扰/锁屏规则及展开偏好。本设计不包含文件上传、网页搜索或设备操作。

## 2. 现状与容量依据

- `companion/mod.rs:449–507` 在观察时以观察描述替换 query；上下文为最近 20 轮、合计 9,000 字符，聊天与观察共用。
- `mod.rs:536–544` 把观察描述写入 `turns.user_text`，以后作为 user/assistant 对重放。
- `mod.rs:559–572` 每 12 轮在请求链中生成摘要，使用本轮入库前的历史。
- `store.rs:81–89` 的 recent 最多返回 200 轮；through_id 存在但未用于选取后续原文。
- `providers.rs` 主要返回文本，缺少统一 usage、结束原因及上下文超限分类。
- `screen.rs` 的最后外部窗口超过 30 秒即不可用，原始截图不落盘。

2026-10-22 18:03（中国时间）以当前账号查询 `GET https://api.deepseek.com/models`：deepseek-flash 显示为 DeepSeek-V4.1-Flash，`context_window=1,048,576`、`max_output_tokens=393,216`。输入输出共享窗口。证据：[模型元数据](evidence/companion-model-limits-2026-10-22.json)。本次只有元数据 GET，没有发送真实聊天、生成长文或测试百万 token 理解质量。

当前对话关闭 thinking，未显式发送 max_tokens；官方非思考模式默认输出为 8,192 tokens。本机回复字符上限 500 与 token 容量不是同一限制。视觉模型独立配置，不能继承该 1M 上限。

依据：[模型详情](https://api-docs.deepseek.com/quick_start/pricing/)、[模型元数据](https://api-docs.deepseek.com/api/list-models/)、[生成参数](https://api-docs.deepseek.com/api/create-chat-completion/)、[token 计量](https://api-docs.deepseek.com/quick_start/token_usage/)、[前缀缓存](https://api-docs.deepseek.com/guides/kv_cache/)。官方别名页的版本后缀不能伪称为本次账号元数据实测返回。

## 3. 数据结构与迁移

### 3.1 对话与稳定顺序

保留 turns.id、原文、译文、sync_state、operation_id，不重新编号既有 Hindsight 文档。新增：

- `interaction_kind`：`chat | screen_question | observation_comment`。
- `finish_kind`：`complete | user_limit | provider_limit`。
- `user_initiated`：用户发送/立即观察为 1，定时观察为 0；旧观察不能区分则允许 null。

新增 `dialogue_order(seq INTEGER PRIMARY KEY AUTOINCREMENT, turn_id TEXT UNIQUE NOT NULL)`，外键引用 turns.id。按 seq 分页，不依赖可复用的裸 rowid；清空后不重置序号分配器。

新数据规则：chat 和 screen_question 的 source=chat、user_text=用户原话；observation_comment 的 source=screen、user_text 为空字符串，只回放 assistant，不虚构用户输入。手动/自动观察的来源由 observation.origin 区分。

### 3.2 独立观察记录

新增 observations，字段至少包括：

- `id`：不复用的观察 ID，新数据使用 UUID，前端按不透明字符串处理。
- `created_by_turn_id`：创建轮次；与 turn 在同一事务提交。
- `origin`：`automatic | manual_once | attached_question | legacy`。
- `observed_at`、`time_basis`：实际捕获时间；time_basis 为 `captured | legacy_turn_time`，旧数据明确为近似时间。
- `scope`：`foreground | screen | region | legacy_unknown`。
- `target_label`：脱敏后的窗口/范围说明，不持久化 HWND、目标票据或完整系统路径。
- `summary`、`evidence_json`：校验后的概括、可见文字、事实和不确定性；结构由配套稿第 7 节定义。
- `confidence`：0–1，旧数据允许 null；模型自评不等于客观准确率。
- `legacy_original_text`：迁移旧观察时保存原始描述，避免原内容丢失。
- `forgotten`：逻辑遗忘标记，遗忘同时清空正文。

原图、base64、视觉原始响应不写库、不导出、不放诊断。校验通过的观察先作为内存 ObservationDraft；最终文字可以提交时再与 turn 一起保存。取消前未提交的观察不悄悄留在后台历史。

### 3.3 引用与参考集

新增 turn_observations，含 `(turn_id, observation_id, relation_kind, ordinal)`，前两项唯一。relation_kind：

- `attached`：本轮用户主动带上新屏幕。
- `explicit`：在界面明确引用已有观察。
- `continuation_candidate`：供自然追问使用的背景，只表示提供给模型，不表示用户确认或模型一定使用。

同轮最多 3 个有效观察。禁止把模型猜出的 ID 直接持久化。新增单例 context_state，保存当前参考集的 ID、强弱来源、锚定用户轮次 seq 和 history_epoch；这不是 session。

引用选择规则：

1. attached/explicit 为强引用，只有用户成功提交的新屏幕或明确引用才替换强引用集。
2. 紧邻观察评论的自然追问可将其作为弱候选；近几轮的真实问答及观察关联可持续提供指代依据。
3. 新定时观察 B 不覆盖正在讨论的强引用 A；必要时二者作为有时间标签的不同来源同时出现。
4. 无人跟进的自动观察初始只取最近 10 分钟内最多 2 次；弱候选闲置 30 分钟降级，不能靠每轮被动注入延长有效期。强引用闲置也必须标为历史资料，不冒充当前画面。
5. 原文或摘要来源中仍有明确 ID 时，过期观察可按 ID 取回；不靠语义搜索猜旧对象。
6. 重启恢复合法引用；遗忘使相关 ID 失效。只有新截图能够支撑“当前屏幕”。

### 3.4 检查点与输入收据

新增 context_checkpoints：版本、history_epoch、previous_version、through_seq、增量起止 seq、源范围摘要、正文/结构、来源引用、token 数、tokenizer/prompt 版本、创建时间及 active 标记。一个活动检查点，有限恢复版本。

through_seq 表示按当前策略处理到的位置。无人跟进观察评论可明确按策略排除并记录排除计数，不声称其内容已总结；用户对话未成功摘要时不能为了省 token 前移水位。

inputs 补充 payload_fingerprint、request_epoch、text_committed、outcome_json。新 client_message_id 用作 input/turn ID，由后端验证规范 UUID 并以唯一键预留。inputs 不是成功对话历史。

存储约束必须落地：turn_observations 的 turn/observation 均有外键和唯一约束；dialogue_order.turn_id 唯一；observations.created_by_turn_id 引用创建 turn；context_state 只有一行；同一 epoch 最多一个活动检查点。保存 turn 后再插 observation/关联，均处于同一事务。索引至少覆盖 seq 分页、observation 时间/有效状态、反向 observation_id 引用和待同步状态。JSON 字段由应用按版本校验，不要求依赖 SQLite JSON 扩展。

payload_fingerprint 对规范化后的脱敏 text、screen 选择和 expected_history_epoch 计算，不记录原始密钥/敏感文本。generation 是当前进程内取消标识，和持久 ID/epoch/seq 分别管理，不能相互代替。

### 3.5 原子迁移

数据库 schema 升为 2，与 AI settings.schema_version 分离。一致性备份后事务迁移，启用外键、必要索引和完整性检查：

1. 按旧 rowid 顺序填充稳定 seq；旧聊天原文、译文、ID、同步状态不变。
2. 每条未遗忘的旧 source=screen 生成确定 ID `legacy:<turn_id>`，把旧 user_text 原样转存 observation；origin=legacy、范围未知、时间近似，不猜手动/自动。
3. 旧 turn 改为 observation_comment、清空伪用户文字、建立引用，旧回复原样保留；已遗忘内容不得恢复。
4. 旧摘要标 legacy，不沿用不可靠的 through_id，按未遗忘原文后台重建；重建期间先用近期原文。
5. 不创建 bank，不重发所有已完成文档；submitted operation 继续跟踪，pending 使用新序列化但保持原文档 ID。

失败回滚并保留备份，不回退默认空库。未知更高 schema 停用 AI 写入并保留数据。新库不承诺旧程序写入兼容；回退须使用匹配备份。重复迁移不得重复观察或序号。

## 4. 共享接口契约 CTX-OBS-1

以下共同接口映射到 Rust serde 与 TypeScript。新前后端同版升级，更新全部调用与测试，不维护两套语义不同的 send 参数。

```ts
type ScreenSelection =
  | { mode: 'none' }
  | { mode: 'fresh'; target_token: string }
  | { mode: 'reference'; observation_ids: string[] };

interface ChatSendPayload {
  client_message_id: string;
  expected_history_epoch: number;
  text: string;
  screen: ScreenSelection;
}

interface SendOutcome {
  request: number | null;
  turn_id: string | null;
  result: 'completed' | 'failed' | 'cancelled';
  text_committed: boolean;
  observation_ids: string[];
  history_epoch: number;
  replayed: boolean;
  error_code?: string;
  warning_code?: string;
}

interface PreparedScreenTarget {
  target_token: string;
  label: string;
  scope: 'foreground';
  expires_at: string;
}

interface RequestReceipt {
  client_message_id: string;
  history_epoch: number;
  state: 'pending' | 'text_committed' | 'finished' | 'interrupted';
  request: number | null;
  turn_id: string | null;
  text_committed: boolean;
  outcome: SendOutcome | null;
}
```

- `companion_prepare_screen()`：仅输入窗；只读目标元数据，返回 PreparedScreenTarget，不截图、不调用模型、不改定时开关。票据规则由配套稿第 5 节定义。
- `companion_send({ payload })`：仅输入窗；三种 screen 均要求非空问题，后端先检查 epoch、字段和去重。
- `companion_observe()`：设置窗的一次手动观察；托盘“立即观察一次”走同一 observe_once；定时任务走 observe_auto，不要求用户文字。
- `companion_get()`：View 增加 history_epoch、最后可引用观察概要、当前请求标识，不返回原图或全部历史。
- `companion_history()`：最近 200 个可见轮次，增加 interaction_kind、finish_kind、observation_refs，观察不复制回 user_text。设置页监听 companion-history-changed 刷新。
- `companion_get_observations({ ids })`：许可本机窗口最多取回 3 个未遗忘观察，不接受 bank/路径/句柄。
- `companion_request_receipt({ client_message_id })`：仅输入窗查询本机收据，返回 RequestReceipt 或 null；不触发截图/生成。重启后的旧 generation 只作记录，不能控制新进程请求。
- companion_stop 和展开/收起保持含义；停止不是遗忘。

内部 `RequestKind = chat | screen_question | observe_once | observe_auto`。不再用 observe 布尔量兼任自动性、观察能力和用户输入替换。fresh 对应 screen_question；reference 属于 chat，不截图。

幂等：相同 ID/payload 已完成只返回收据，不截图/生成/播音/同步；pending 报 request_in_progress；相同 ID 不同 payload 报 request_id_conflict。失败后的用户重试使用新 ID。epoch 不匹配报 history_changed，刷新后再发。崩溃遗留 pending 标 interrupted，重启不自动重放或截图。

前置校验失败可返回 Tauri Err；接受后的结束均返回 SendOutcome。文字成功、语音失败仍 completed 并附 warning；提交后停止为 cancelled 且 text_committed=true，不能撤销已完成 turn。

## 5. 请求流程与事务边界

chat：校验 → 上下文 → 一次回答 → 提交 turn/引用/队列 → 完成事件 → 可选语音。

screen_question：校验目标 → 一次新截图 → 针对问题的观察 → 统一上下文 → 一次回答 → 同事务提交 observation/turn/引用/队列 → 完成事件 → 可选语音。捕获、视觉、失败细节由配套稿第 5–8 节定义。

observe_once/observe_auto：一次截图 → 通用观察 → 一次短评论 → 同事务提交 observation 和只有 assistant 的 turn。自动任务受定时开关、阅读/播放租约和忙碌约束；手动任务不启用定时开关。

共同不变量：

- 生成前只保存 pending 输入收据，ObservationDraft 仍在内存。
- 文字有效、请求仍当前、epoch 未变且访问仍允许时，短事务插入 observation（如有）、turn、seq、引用、待同步状态和文字提交收据。
- 成功提交后才发成功终态和历史变化，再启动语音；不能先宣称保存成功。
- 提交前取消/超时/失败不保存 turn/新观察，不入 Hindsight；可见半截用错误终态释放队列，保留草稿。首版不将取消片段当后续上下文。
- 提交后停止/语音失败保留文字与观察。模型使用已保存全文，即使气泡未读完；不重复显示观察评论。
- provider_limit 有正文可保存为未完成回答并提示继续，摘要不能当完整承诺；应用字符上限为 user_limit。摘要本身 length 截断绝不能提交。

## 6. 总预算与分流选择

### 6.1 token 预算

所有 K=1,024 tokens，计入实际发出的消息、包装、角色及来源标签。

- C=1,048,576，O=8,192，S=32,768，输入硬保护 H=C-O-S=1,007,616。
- max_tokens 应与 O 一致；允许更长输出同步减少输入，500 字符不能当 500 tokens。
- 初始常态目标 128K，后台压缩触发 192K，摘要未完成的日常应急上限 256K，明确长历史引用的单轮扩展上限 768K。
- 所有限额取与 H 的较小值；较小/未知模型按 `(base_url,model)` 能力调整，不能继承旧模型的 1M。

这些是可调策略，不是官方限制或实测最优值。[预算算术](evidence/companion-context-budget-check-2026-10-22.json)不是生产实现或性能测试。

### 6.2 选择顺序与观察额度

先保护系统规则和本轮原话，再优先最近完整问答、明确引用的原文/观察，随后分配摘要、连续对话及辅助资料。通常至少保留最近 4 轮用户问答；极端超长条件下它是目标，不作无条件保证。

主对话含 chat、screen_question 及用户明确引用观察对应的主动发言。无人跟进自动评论作为少量辅助候选，不与用户问答等权竞争。候选最多 3 组；通用观察初始上限 8K tokens，Hindsight 事实 4K，确定性取回的旧原文 16K；未用额度留给对话，明确引用可借用总额度。

正在讨论 A 时新 B 不替换 A。当前图、旧观察、用户自述和 AI 推测分别标注。只保留文字意味着无法恢复未记录像素。自然指代不新增前置模型：提供近期原文、已知引用、有限候选，由最终模型理解；仍不明确时澄清。

### 6.3 消息顺序与计量

推荐：稳定人格 → 有效摘要 → 时间顺序的真实角色原文 → 本轮旧原文/观察/Hindsight/时间天气资料 → 用户问题。

观察评论按 assistant 重放，观察材料在资料区。动态资料不混入固定人格，所有来源视为数据而非指令；缓存效果以 usage 为准。

tokenizer 须与当前别名兼容，缓存不可变原文计数并补足模板；旧离线 tokenizer 不能直接视为新模型精确分词。usage、finish_reason、缓存量向上返回；主动停止拿不到 usage 时标未知，不能写 0。估算不精确时提早压缩并保留余量。

## 7. 后台增量压缩与截断

接近 192K 时压缩最老可处理前缀。最多一个后台工作器，不再每 12 轮同步摘要，不占聊天 busy 或 Hindsight memory_gate。

1. 固定快照：旧检查点＋旧水位之后增量＋必要观察资料；不把 profile 反复混入旧摘要。
2. 按约 32K–64K 输入 tokens 分块；普通问答不拆，超长单轮按段落/代码块切分，保留共同源 ID。
3. 摘要初始上限 8K，保留用户更正、未完成话题、承诺、指代、编号结构与源 ID。
4. 核验 JSON/结构、输出完成状态、长度、源 ID 和覆盖范围后，事务更新正文和水位，逐步回到约 128K。覆盖范围由程序确定，禁止硬截 JSON。
5. 默认仅加入水位之后原文；明确引用已摘要原文可按 ID 带回，去重并标来源。

新消息追加不必使前缀任务作废；提交核对前缀未变、前版本和 epoch。遗忘/清空变更 epoch 使旧任务失效。await 不持有数据库/全局设置锁。摘要有损，结构检查不能证明语义无遗漏。

失败保留旧摘要、水位和全部原文；可短暂增长至 256K。仍未准备好时，本轮省略较早低优先级原文，首次提示“本轮使用近期对话，较早记录仍保留”，不能声称已成功总结。

替换现有“最近一轮过长就 break，历史全空”：先局部节选/摘要或扩展；保留当前问题；必要资料自身超上限时要求分段，不静默删除。明确超限且尚未显示文字时最多重组重试一次，不把任意 400/鉴权/余额/网络错误归为超限；已显示后不自动重播。重试复用同一次观察，不能抓第二张图。

## 8. Hindsight、历史、导出与遗忘

### 8.1 公共同步序列化

保留 bank/owner、稳定文档 ID、异步 operation 和退避。正常同步、重试、遗忘后重建必须走同一序列化函数：

- user 段只有真实文字，问题/假设不自动当声明。
- 观察单列“某时屏幕资料”，含 ID、时间、来源和不确定性；主动观察明确不存在用户自述。
- 新 screen_question/observation_comment 不把 AI 的屏幕答案当独立用户事实反复提取；主要保留真实用户声明、有效观察和必要交谈关系。普通 chat 的 AI 表达仍明确非用户声明。
- 观察材料并入相关 turn 的稳定文档，首版不另建重复 observation 文档；不要同步整份当前上下文。
- 敏感、失败、提交前取消、状态 JSON 和纯技术错误不入队；provider_limit 片段不当完整承诺。
- memory_enabled 关闭时不召回/同步，也不注入 Hindsight 派生 profile；本地原文与摘要继续有效。

### 8.2 历史与导出

设置页仍按时间显示，无 session 列表。区分真实问题、主动发言、屏幕提问和结束状态，观察标签可展开文字。监听历史变化刷新，不用旧列表判断漏记。

导出为 `schema_version=2` 的对象，含 dialogues、observations、references；观察正文只导出一次。保留源 ID、时间、近似时间标记和 legacy 原文，排除遗忘内容、密钥、票据、原图和临时响应。不再输出无版本的裸数组。

### 8.3 遗忘范围

“忘记这条对话”：移除指定 turn 正文/引用，遗忘该轮创建的观察并清除相关候选，使其他轮次对该观察的引用失效；不批量删除其他已经存在的对话正文。界面说明这个范围，不宣称已从所有对话抹除相同事实。

“忘记这次观察及相关对话”：先展示后端计算的直接关联轮次数并确认，再统一遗忘该 observation 及直接引用它的轮次。被删轮次创建的其他观察一并失效，但不递归删除更多未选对话。两种操作分别命名，不悄悄扩大范围。

保留现有 `companion_forget({ id })` 的单条/全部入口，后端实施上述新语义；新增设置窗专用 `companion_forget_observation_preview({ id })` 返回 observation ID、受影响对话/观察数量、history_epoch、短期 preview_token，及 `companion_forget_observation({ id, preview_token, expected_history_epoch })`。后者在事务内重算范围；若其间出现新引用或 epoch 变化，报 deletion_scope_changed/history_changed，重新展示范围，不按旧确认扩删。预览票据不保存观察正文，也不授权其他 bank 操作。

两者均在事务内递增 history_epoch、清理派生资料/收据/引用、废弃摘要和后台草稿，并标记原 bank 待重建；旧远端检索暂停。剩余轮次序列化不得读取已删除观察，保留的其他用户原文不改写。清空删除所有正文、观察、关系和缓存，保留原身份及序号不复用规则。

取回、最终提交、摘要及同步提交前复核 epoch。普通追加不改 epoch；单条遗忘后仍保留的旧 observation 不能仅因创建 epoch 较旧就被误判失效。备份不是模型来源，不承诺处理用户自备副本的物理擦除。

## 9. 状态、模块与现有行为

companion-status 增加 request（后台为 null）、client_message_id、state_seq、stage、error_code/warning_code、text_committed，保留 message/busy。stage 仅为 `preparing | capturing | analyzing | context | replying | text_committed | translating | synthesizing | finished`。后台记忆/摘要状态不覆盖前台请求。

companion-reply 保留累计 text、request、id、done/error，增加 interaction_kind/finish_kind；screen_question 的兼容 source 为 chat。audio 仍绑定同一 turn/request。按 request 和递增 state_seq 丢弃迟到状态，旧 finally 不能清掉新 busy。

文件分工：context.rs 管预算/选择/摘要；observations.rs 管 DTO/引用/材料，screen.rs 管捕获；store.rs 管迁移/事务/遗忘；memory.rs 管统一序列化；mod.rs 管生命周期和工作器；providers.rs 管 usage/结束原因/错误。前端 types、输入页、设置页和 presentation 同步更新。

保留窗口查询锁顺序：释放 storage/数据库 guard 后才调用可能等待 UI 线程的窗口 API。收起不中断请求，静音仅影响声音，锁屏/免打扰/隐藏按原访问规则失效。

## 10. 联合实施与验收

两功能在同一分支完成契约对齐后发布，不把新观察临时写回旧 user_text 上线：

1. 先共享 DTO、schema/seq/epoch/引用与事务。
2. 把普通聊天、手动/自动观察迁入 RequestKind。
3. 实现 token 预算、资料分流、后台压缩及回退。
4. 按配套稿实现目标准备、fresh/reference、针对问题识图，接统一上下文。
5. 完成历史/导出/遗忘/状态过滤，再联合验证。

验收编号：

- **CTX-01**：迁移可重试，无原文/译文/ID 丢失；旧观察变为独立记录及 assistant 发言，无伪用户，无 bank 文档重复。
- **CTX-02**：重启、切模型持续同一对话，无新增 session/RAG。
- **CTX-03**：大量观察不挤用户问答，A 不被新 B 自动改指代，歧义可澄清。
- **CTX-04**：问题和观察分存；事务失败、取消、epoch 改变无孤立 observation/turn/outbox。
- **CTX-05**：全部消息计量，输出预留/小窗口满足硬保护，超长最近单轮不导致原文全空。
- **CTX-06**：摘要按 token 触发，水位与提供原文一致，第 12/24 轮无特殊漏记，不反复压最新 20 轮。
- **CTX-07**：摘要超时、截断、无效源 ID/结构不提交，前台仍可继续。
- **CTX-08**：按 ID 找回原句/观察，角色时间正确，旧图不冒充当前，不补造像素。
- **CTX-09**：单条/关联/全部遗忘按已展示范围，迟到生成/摘要/同步不能回填。
- **CTX-10**：重复请求只返回收据；提交后语音失败不重发，提交前失败保留草稿。
- **CTX-11**：历史和导出 v2 对来源、近似时间、部分回复及遗忘一致，列表随事件刷新。
- **CTX-12**：长期记忆关闭/离线时本地可聊，profile 不绕开开关，正常/重建采用相同来源规则。
- **CTX-13**：窗口查询锁、静音、锁屏、阅读租约、收起和迟到状态无回归。
- **CTX-14**：合成对话覆盖更正、承诺、编号、观察续聊；测首字/完成延迟、tokens、缓存与摘要额外用量，阈值不当性能承诺。

SCREEN-* 联合验收见[配套稿第 11 节](companion-screen-question-design-2026-10-22.md#11-验收要求)。元数据和预算算术不能替代实现后的语义、并发和实机测试。
