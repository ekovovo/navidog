# 架构基础优化第一批验收

日期：2026-10-21。基线：`upstream/main` 的 `4c403de`；分支：`codex/architecture-foundations`。规格先提交为 `4d09157`，边界补充见 `8bc7a6e`、`cf9e7b3`。本分支基于 main 现有 M1–M3，未合入 M4 PR。

## 已完成范围

1. `34702fa`：普通 SQL 每次最多执行一次，传输失败不自动重放；应用元数据通过显式安全读取允许一次恢复，普通 SQL 会话保守保持物理会话依赖。重连更新稳定取消目标，关闭/失效后清除。包含 MySQL、MariaDB、PostgreSQL 主动断连、已提交写入响应丢失、事务丢失与旧取消句柄的实库检查。
2. `5fc7866`：连接配置采用候选保存事务，成功落盘后才更新内存/密钥和发布事件；编辑、复制、删除、分组及批量操作失败回传。删除的关闭会话/清理目录，以及配置文件切换的重连，都在保存成功后执行。测试覆盖激活与非激活配置文件内全部秘密、写入阻断、失败重试与事件次数。
3. `023d3c4`：GridModel 唯一拥有当前结果行与隐藏列，排序/投影不再用旧快照覆盖已提交值；结果来源在运行开始时固定，固定结果切换连接后仍按原来源提交和同步，匹配会话保留临时对象与事务上下文。包含隐藏/恢复列顺序回归，并移除整份 ResultSet 的多余克隆。
4. `ace88be`、`b1568e0`：复核后补齐空白新增行隐藏/恢复列的默认值状态；跟踪已识别 `USE` / `SET search_path` 后的服务器实际来源，独立会话同步恢复完整 PostgreSQL 路径。探测失败保留原 SQL 成功，不重放；来源未知时仅复用同一上下文，拒绝独立会话同步。实库断言确认同步后修改覆盖层已清除。
5. `66f9391`：修复全量审计发现的基线证书依赖漏洞，仅更新 `rustls-webpki` 0.103.15 → 0.103.16 与 `webpki-roots` 1.0.9 → 1.0.10；未增加审计忽略。

## 初版与 CI 修复后的本机验证（2026-10-21）

| 检查 | 结果 |
| --- | --- |
| 格式、clippy、裸值和 diff 空白检查 | 通过；依赖保留既有 future-incompatibility 提示 |
| `NAVIDOG_LIVE=1 cargo test --workspace --locked` | 215 项，33 个测试/文档测试套件全部通过，无失败或忽略 |
| DB 实库 | 14 项通过，含原有协议/TLS/SSH/空闲恢复与新增故障注入 |
| 查询测试 | 29 项通过，含双 SQLite 来源隔离、固定结果、临时视图、运行中改目标、三种实库的 SQL 上下文与探测失败 |
| 截图 | 29 张，浅深色逐张检查；新增配置保存失败原图两张随代码归档 |
| 依赖审计 | `cargo deny --all-features check` 四项全部通过；`cargo audit` 在线更新后通过，保留既有 9 条允许的间接依赖 unmaintained 警告。RustSec 数据库为 `e6f94f37`（2026-10-20），已包含 RUSTSEC-2026-0382、RUSTSEC-2026-0381 |

首次审计命中的两项基线漏洞已通过兼容补丁更新修复，并重新在线审计。验证使用仓库固定的 Rust 1.99.0 / gpui-kit 0.8；复现记录保存在本地 `target/architecture-checks/`，构建产物不进 Git。实库容器在验证后关闭，不影响其他本机数据库。

截图和复现说明见 [截图集](https://github.com/xhofe/navidog/blob/c76bc4c9c14a90868d544b9724ef37c4fad8dbf6/docs/screenshots/architecture/README.md)。本机渲染不代替 Navicat 运行态并排验收。

## CI 稳定性补验

PR #14 的原始提交 `be309f0` 在 Intel macOS 的[一次 CI 运行](https://github.com/xhofe/navidog/actions/runs/39535956784/job/119012529027)中于 `info_tests::query_pages_follow_the_editor` 保存代码段失败，同一提交的另一次 Intel 运行通过。测试未关闭对话框入场动画，`window.click` 在两帧之间取坐标并命中时可能落到移动后的其他位置；打开与输入也挤在同一个窗口更新内。这与 AGENTS 记录的动画和通知时序问题一致。

修复仅调整测试：初始化启用 reduced motion，打开对话框后先完成一轮调度，再执行真实点击和输入；保存前断言名称为 `films` 且名称框拥有焦点，保留原有保存结果和数据库类型断言。未增加重试点击、等待保存或跳过测试。该用例本机连续 20 次独立运行通过；其余复验和远端平台检查见 [PR #14](https://github.com/xhofe/navidog/pull/14/checks)。

修复后本机重跑全工作区实库模式仍为 215 项、33 个套件全部通过；fmt、裸值检查、clippy、deny、在线 audit、29 张截图及三项 release 性能检查均通过。归档的浅深色失败截图与本次生成结果逐字节一致。实库容器已关闭，记录保存在 `target/architecture-checks/ci-fix-*.log`。

## PR #14 review 修复（2026-10-22）

本轮按作者的八条评论补充实现与回归，规格先提交为 `a3bdf52`。推送前合入作者新增的 `9c69bd1`、`0c9f525`，保留其新增回归、公开接口、分组缺失 ID 拒绝，以及独立操作会话中的 MySQL 数据库上下文准备；合并结果重新全量验证。下表记录当前行为及可运行的检查入口；上面的 215 项测试、29 张截图和后面的性能数字属于上一轮验证，不代表本轮最终结果。

| 评论 | 修复后的行为 | 回归入口 |
| --- | --- | --- |
| [4332969362：未提交时排序箭头与行不一致](https://github.com/xhofe/navidog/pull/14#discussion_r4332969362) | 查询结果先结算正在输入的单元格；有待提交修改或空白新增草稿时拒绝排序，显示本地化提示，保留行与上次接受的排序箭头。表头内部记录的排序也恢复，下一次点击仍按原顺序切换。 | `navidog-query` 的 `editor_tests::accepted_results_keep_their_rows_and_original_source`：点击真实表头，分别覆盖脏值、空白草稿及拒绝后的后续排序。 |
| [4332969373：完成写入污染元数据会话恢复](https://github.com/xhofe/navidog/pull/14#discussion_r4332969373) | 查询结果提交、导航器 DDL/复制表和连接 FLUSH 通过 `Sessions::run_operation_rel` 使用独立短期会话；闭包只执行一次，结束后关闭。关闭失败只记日志，保留原操作成功或失败；共享元数据会话保持安全读恢复能力。表查看器仍使用自己持有的会话。 | `navidog-db` 的 `sessions::tests::operation_cleanup_preserves_success_and_failure_without_replaying`；实库 `sessions::tests::completed_operation_does_not_pin_or_replay_on_metadata_recovery`（MySQL/MariaDB/PostgreSQL，需 `NAVIDOG_LIVE=1`）；查询编辑器原来源回归另保留一个元数据事务，确认结果提交不会进入该事务。 |
| [4332969383：数据库层写死英文恢复引导](https://github.com/xhofe/navidog/pull/14#discussion_r4332969383) | `DbError` 保留驱动原始错误码和消息，单独提供会话丢失、执行结果未知状态。网格、查询、导航器和连接 FLUSH 在各自界面边界使用 en/zh-CN 文案；连接建立失败不误报写入结果未知。 | DB 的 `execution::tests::session_loss_keeps_the_original_database_error_and_structures_its_outcome`、现有 `uncertain_writes_are_never_replayed` / `transaction_loss_cannot_reconnect_through_metadata_or_ping`；导航器 `errors::tests::recovery_guidance_is_localized_without_changing_the_driver_diagnostic`；连接 `reload::tests::flush_recovery_guidance_keeps_diagnostics_and_uses_the_interface_language`。 |
| [4332969392：缺失 ID 静默成功](https://github.com/xhofe/navidog/pull/14#discussion_r4332969392) | 单个或批量更新遇到任意缺失 ID，先返回 `StoreError::NotFound`，不运行任何修改回调、不保存、不发布 Changed；空批量成功但不执行回调或保存。 | `navidog-core` 的 `store::tests::missing_update_targets_are_rejected_before_any_callback`：覆盖缺失 ID 在批量前、中、后的情况，核对回调、事件、内存与原磁盘内容。 |
| [4332969398：保存密码失败阻断本次连接](https://github.com/xhofe/navidog/pull/14#discussion_r4332969398) | 保存失败后仍以本次输入的内存凭据继续连接，关闭密码对话框并仅调用一次连接回调；通知明确密码没有保存，原持久配置保持不变。 | `navidog-connection` 的 `secrets::tests::failed_password_save_still_uses_credentials_once`：制造写入失败，核对一次回调、收到的凭据、对话框关闭和持久状态。 |
| [4332969404：配置错误混用中英文且 feature 无法复用](https://github.com/xhofe/navidog/pull/14#discussion_r4332969404) | StoreError 的本地化映射移到 `navidog-core`，连接与导航器使用同一入口；日志 Display 和界面文案分开，不增加 feature crate 互相依赖。 | `navidog-core/tests/store_errors.rs` 的 `store_errors_have_english_and_chinese_messages`，检查错误变体在两种语言下的文案及原诊断保留。 |
| [4332969407：隐藏列拒绝结果被丢弃](https://github.com/xhofe/navidog/pull/14#discussion_r4332969407) | `ResultGrid::set_hidden_columns` 返回成功或拒绝，查看器只在成功后更新隐藏列菜单状态。拒绝时保留列、活动编辑器与待提交值；空白新增草稿仍可投影。 | `navidog-grid/tests/grid.rs` 的 `refused_column_visibility_keeps_pending_values_and_the_editor`；查看器 `viewer::ownership_tests::visibility_uses_accepted_updates_inserts_and_deletes`。 |
| [4332969410：刷新静默丢失修改](https://github.com/xhofe/navidog/pull/14#discussion_r4332969410) | 已交付的生产路径是提交后同步：先接受已完成的写入，再按原来源重载；后续读取失败不保留可重复提交的旧修改。当前没有查询结果手动刷新用户入口；内部刷新函数结算活动单元格并拒绝待提交修改和空白草稿，只约束未来调用，不代表已交付交互。 | 查询编辑器原来源回归检查同步来源和触发器计数；同步读取故意失败后，再次提交不会产生第二次写入。活动输入、空白草稿和拒绝时不发送 SELECT 的断言通过测试包装调用内部守卫，不能证明用户入口存在。 |

此外，导航器右键菜单新增“重新打开连接”，断连操作错误也能引导到原连接的同一动作。确认文案说明事务、锁、临时对象无法恢复，已执行语句的结果可能未知；取消不关闭会话。确认后复用 `SessionsEvent::Reopening`，保留编辑器选项卡、查询文本及本地行草稿；再次提交前需要核对服务器状态，不能把重新打开理解为恢复旧事务或安全重试。回归入口为 `navidog-connection/tests/reopen.rs` 的 `reopening_requires_confirmation_and_preserves_the_editor_lifecycle`，以及导航器 `ops::recovery_tests::session_loss_error_offers_reopen_for_the_original_connection_after_dismissal`。查看器原来源回归实际替换会话，确认草稿不丢失、不自动提交，用户明确 Apply 后才写入；查询编辑器原来源回归通过真实文本输入产生未保存文档，确认重开前后文档、固定结果及空白新增草稿保留。

### PR #17 首轮提交验证（`c76bc4c`）

- 最终代码以 `NAVIDOG_LIVE=1 cargo test --workspace --locked` 通过 232 项、37 个测试/文档测试套件，0 失败/忽略；其中 DB 为 27 项 lib、15 项 live、2 项 SQLite，query 31 项。新增操作会话回归位于 lib 内，实测 MySQL/MariaDB/PostgreSQL 写入后元数据恢复；CI integration 同步执行 `cargo test -p navidog-db --lib --test live --locked`。显式重开的查看器及查询草稿保留断言也包含在最终全量结果中。
- fmt、全工作区全目标 clippy（拒绝警告）、裸值与 diff 空白检查通过。在线 `cargo audit` 使用 RustSec `4b587f00`（2026-10-20，1348 条通告），无未允许的漏洞；`cargo deny --all-features check` 的 advisories/bans/licenses/sources 全部通过。保留既有 9 条允许的间接依赖 unmaintained 警告和原有 future-incompatibility 提示，未新增忽略。
- 37 张新生成的浅深色截图已逐张检查；新增重开确认两张原图随代码提交，见 [截图集](https://github.com/xhofe/navidog/blob/c76bc4c9c14a90868d544b9724ef37c4fad8dbf6/docs/screenshots/architecture/README.md)。既有保存失败两张原图与本次生成结果逐字节相同。截图不代替 Navicat 运行态并排验收。
- 三项 release 性能复验通过：10 万行 × 20 列网格逐行/跳跃平均 3.35/4.80ms；导航树 0.54/0.74ms；5 万行 SQL 的滚动、装饰及输入平均 1.63～1.67ms，首次解析 340ms；各场景均 0/300 帧超过 8.3ms。以上仅为本机测量。
- 本轮复现日志保存在 `target/pr14-review-checks/`；实库容器验证后关闭。PR #14 已合并，本轮补充从最新 main 的 `9f3a926` 整理到 `codex/pr14-review-followup`；保留已合入的 PR #15 底部 SQL 修复。远端 CI 以本轮后续 PR 的最新提交检查为准；上一轮已通过的 CI 不作为本轮结果。

## PR #17 AI 与作者 review 修正（2026-10-24）

规格先提交为 `57efe56`、`35539cc`。本轮按五条 AI 评论及七条新增作者评论复核兼容性、操作生命周期、空白草稿与本地化；手动刷新问题采用收窄声明的处理方式，不新增菜单、快捷键或按钮。上面的 232 项测试、37 张截图和性能数字属于 `c76bc4c` 首轮提交，不能作为本轮修改的最终验证结果。

| 评论 | 处理与状态 |
| --- | --- |
| [4350078492：旧配置错误格式化入口删除](https://github.com/xhofe/navidog/pull/17#discussion_r4350078492) | 已恢复 deprecated `navidog_connection::store_error_message`，保持旧 import 路径和函数签名，委托 `StoreError::localized_message`；新增外部集成测试覆盖英中映射与 core API 一致性；外部兼容检查已加入全量测试。 |
| [4350078505：DbError 结构体调用不兼容](https://github.com/xhofe/navidog/pull/17#discussion_r4350078505) | 移除新增的 `#[non_exhaustive]`，保留 main 现有公开字段及构造、解构方式；新增外部 API 兼容检查；外部兼容检查已加入全量测试。 |
| [4350078520：关闭或重开后旧操作仍执行](https://github.com/xhofe/navidog/pull/17#discussion_r4350078520) | 按连接代际登记并撤销操作，等待实际初始化、取消和清理；SQLite 阻塞任务登记到实际工作线程退出。重开等待旧任务，批量语句和界面回调均核对代际；已知结果保留，无法确认的已开始操作保留结果未知语义。含三驱动实库防重放、门闩与 SQLite 排队回归。 |
| [4350078530：接受提交时丢弃独立空白草稿](https://github.com/xhofe/navidog/pull/17#discussion_r4350078530) | 用仅含数量与选区的移动令牌保留空白草稿，覆盖关闭同步、同步成功和失败，保持 DEFAULT 语义。SQLite 触发器核对更新、插入、删除只执行一次；重复同步不增加草稿，明确放弃仍清除。 |
| [4350078539：手动刷新没有用户入口](https://github.com/xhofe/navidog/pull/17#discussion_r4350078539) | 已收窄规格与验收声明：当前查询结果没有手动刷新用户入口，内部守卫只是未来调用约束；本轮交付的是提交后同步，保留原来源、排序和已完成写入不重放的回归事实。未把内部测试调用当作用户交互验证。 |

### 新增作者评论

| 评论 | 处理与验证 |
| --- | --- |
| [4351271592：DB 错误只有中文](https://github.com/xhofe/navidog/pull/17#discussion_r4351271592) | DB 接入 rust-i18n，两个非关系型会话分支共享 `database.not_relational` 的 en/zh-CN 文案；新增的操作撤销错误也补齐两种语言。外部假会话与退休文案测试验证翻译。 |
| [4351271600：run_once 中转接口](https://github.com/xhofe/navidog/pull/17#discussion_r4351271600) | 将既有公开入口明确标为 deprecated，仅作为下游源码兼容 shim；内部统一使用 `run_operation_rel`。新增外部调用测试证明保留理由，不增加第二套执行逻辑。 |
| [4351271607：恢复文案分支重复](https://github.com/xhofe/navidog/pull/17#discussion_r4351271607) | `navidog_ui::db_error_text_with` 统一原始诊断格式及 unknown/session-lost/raw 分支；四个界面保留各自翻译。共享分支测试覆盖优先级和原始消息不变。 |
| [4351271611：query 借用 viewer 文案](https://github.com/xhofe/navidog/pull/17#discussion_r4351271611) | query 新增专属 `query.session_lost` / `query.outcome_unknown`，消息、摘要、提交、同步、命令列、解释与运行全部改用自己的 formatter；双语测试检查 query 特有措辞。 |
| [4351271618：忙碌误报为有草稿](https://github.com/xhofe/navidog/pull/17#discussion_r4351271618) | 排序只因待提交值或空白草稿拒绝；干净的同步读取中仍接受排序，并在返回后重用。内部刷新守卫先结算和检查草稿，再避免并发读取。原来源 UI 回归新增真实加载中的排序检查。 |
| [4351271623：无会话时改为整批失败](https://github.com/xhofe/navidog/pull/17#discussion_r4351271623) | 恢复既有无会话时跳过该条语句的规则，移除新增提示文案；只有启动后连接代际改变时撤销旧命令，防止余下语句进入新连接。 |
| [4351271633：NotFound 未带目标 ID](https://github.com/xhofe/navidog/pull/17#discussion_r4351271633) | 纠正规格中的过度声明：沿用 main 既有无字段 `StoreError::NotFound`，本轮验证的是完整目标集合在回调、写盘和事件之前拒绝，不声称新增 ID 诊断，也不改变公开变体。 |

### 本轮最终验证

- `NAVIDOG_LIVE=1 cargo test --workspace --locked`：249 项、39 个测试/文档测试套件通过，0 失败、0 忽略。DB lib 34 项包含初始化/执行/清理门闩、SQLite 工作线程清理，以及 MySQL/MariaDB/PostgreSQL 已完成写入不重放的真实回归；另有 15 项 live、2 项 SQLite、3 项外部 DB API 检查。query 32 项包含干净读取中的排序、三种同步路径保留空白草稿与旧代际回调拒绝。
- fmt、全工作区全目标 clippy（拒绝警告）、裸值与 diff 空白检查全部通过。在线 audit 使用 RustSec `84d943a6`（2026-10-23，1351 条通告），deny 的 advisories/bans/licenses/sources 全部通过。
- 审计新发现的 RUSTSEC-2026-0385、RUSTSEC-2026-0386 分别通过 `ctutils 0.4.3 → 0.4.4`、`tokio 1.53.2 → 1.53.3` 修复；仅更新这两项补丁和 DB 本地化依赖，未新增审计忽略。既有 9 条允许的间接依赖 unmaintained 警告与 future-incompatibility 提示保持可见。
- 截图 harness 成功生成 37 张 PNG，已逐张检查浅深色界面，并放大复核重开确认、批量删除窄窗口与全文悬停。新测试截图保留在本地 `target/snapshots/`，不提交到 Git；PR 描述保留已有供审阅的图片链接。截图不代替 Navicat 运行态并排验收。
- 三项 release 性能复验：10 万行 × 20 列网格逐行/跳跃平均 3.57/4.76ms；导航树 0.56/0.76ms；5 万行编辑器滚动、装饰和输入平均 1.65～1.72ms，首次解析 353ms。每个场景均为 0/300 帧超过 8.3ms；这些是本机测量。
- 实库容器已停止。本轮日志在 `target/pr17-ai-review-checks/`；远端结果以 PR #17 最新提交的 CI 为准，本机验证不代替其他平台的 CI。

## 与最新 main 的冲突同步（2026-10-24）

在 `934e76a` 上合并 `upstream/main` 的 `4ffa185`，保留已合入的 M4 表设计器及 PR #18 查询执行联动刷新。此次使用合并提交，不重写已经回复 review 的提交。

- 关闭/重开时同时清理操作任务、取消句柄、事务等待、待刷新通知和待提交的导航动作，草稿仍保留。
- 表数据、计数及目录结果应用前验证原连接代际；自动刷新同时校验原会话，沿用本界面的会话失效及未知结果提示。新旧 `apply_page` 签名已统一。
- 单一 ownership 回归先验证 #17 的隐藏列拒绝、空白草稿和旧代际回调，再重置数据库 fixture，继续运行 #18 的通知合并、字段/外键更新、编辑及事务保护。另用真实会话锁把自动刷新排队，随后重开连接，验证旧任务不能覆盖新查看器。
- 保留 main 的 `ctutils 0.4.5` 与 `tokio 1.54.1`；在线审计新发现的 RUSTSEC-2026-0387 用 `crypto-bigint 0.7.6` 修复，不增加审计忽略。
- M4 仍沿用 main 的既有设计器实现；本轮不把其所有保存流程声明为已经完成 #17 的操作会话生命周期改造。

最终依赖上的验证（Rust 1.99.0，Apple Silicon）：

- `NAVIDOG_LIVE=1 cargo test --workspace --locked`：284 项、43 个测试/文档测试套件通过，0 失败、0 忽略；含三服务端、TLS/SSH、连接代际、查询/控制台窗口关闭通知及表查看器合并回归。
- fmt、全工作区全目标 clippy（拒绝警告）、裸值和差异空白检查通过。在线 audit 与 deny 的 advisories/bans/licenses/sources 通过，仍显示既有 9 条允许的间接依赖维护告警，未新增忽略。
- 显式运行 snapshot harness 生成 58 张 PNG，逐图检查浅深色页面，另放大复核重新打开确认与自动刷新草稿提示；图片保留本地，不提交 Git。这些 headless 截图不代替 Navicat 真实窗口并排验收。
- release 性能：10 万行 × 20 列网格逐行/跳跃平均 3.08/4.39ms；5000 项导航树 0.55/0.75ms；5 万行编辑器滚动、装饰及连续输入平均 1.54～1.63ms，首次上色 346ms；所有场景均为 0/300 帧超过 8.3ms。
- 实库容器已停止。日志为本地 `target/pr17-merge-*.log`；远端 CI 仍需针对本次合并提交单独确认。

## 保留边界

- 任意普通 SQL 执行过后，会话不自动重连；传输失败必须明确重新打开，避免丢失事务、锁、临时对象或未知会话设置。
- 仍为全量内存结果，不引入流式协议；提交后同步跨来源使用新会话，只保证原连接/数据库/模式，不复制旧会话临时对象。
- SQL 实际来源跟踪覆盖既有识别的 `USE` / `SET search_path` 形式；不推断任意函数、`SET LOCAL` 或其他未识别命令内部的上下文变化。
- 编辑器跨窗口宿主协议、完整配置版本迁移、元数据缓存统一与 M5 注册入口不在本批范围。
- 独立操作会话不继承查询编辑器中手动开启的事务、锁或临时对象；查询结果提交仍是完整独立事务。来源匹配的结果同步继续复用原编辑器会话。
- 当前没有查询结果手动刷新的菜单、快捷键或按钮；内部刷新守卫的测试只证明未来调用的拒绝规则，不能视为可供用户使用的功能。编辑器的 ⌘R 仍为运行查询。
- 显式重新打开只建立新会话，不恢复旧事务、锁、临时对象，也不重放结果未知的操作；用户应先核对服务器上的实际结果。

## 初版性能复验（2026-10-21）

三项 release 程序均在初版及 CI 修复后的代码上复验完成：10 万行 × 20 列网格逐行/跳跃均值 3.15/4.46ms，导航树约 0.54/0.73ms，5 万行 SQL 编辑器滚动/装饰/输入各场景均值约 1.59～1.81ms，首次解析约 347ms；各场景均 0/300 帧超过 8.3ms。这是本机测量，不代表任意硬件保证。
