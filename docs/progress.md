# 实现进度

本文件记录各里程碑和各功能的实现状态，是唯一的进度来源。功能的定义见 [功能清单](design/01-feature-inventory.md)，里程碑的定义见 [总览](design/00-overview.md#6-里程碑)。

**维护规则**
- 每个 PR 都要同步更新本文件：改动里程碑状态、功能状态，以及“临时偏差与遗留”一节。
- 状态只用四种：`完成`、`部分`（写明缺什么）、`进行中`、`未开始`。
- 某个功能拖到后续里程碑时，在“说明”列写明目标里程碑，并在功能清单中同步修改。
- “验证”一栏写明是哪个测试或检查证明了它，无法自动验证的写“目测”。

## 1. 里程碑

| 里程碑 | 状态 | 分支 | 规格 | 说明 |
| --- | --- | --- | --- | --- |
| M0 参考资料与设计文档 | 完成 | `m0-reference-and-specs` | 00 ~ 03、10、11 | |
| M1 应用外壳和连接管理 | 完成 | `m1-shell-connection` | 10-shell、11-connection | 部分 token 仍是暂定值，待 AX 实测（见第 3 节） |
| M2 对象树、对象选项卡、表查看器 | 进行中 | `m2-specs`、`m2-objects-grid` | 20-objects、21-table-viewer | 任务都已完成，剩两份规格验收清单里的并排目测 |
| M3 查询编辑器 | 进行中 | `feat/m3`、`codex/m3-completion` | 30-query-editor、31-query-results | 运行/选择/当前语句/停止、存储过程、结果上限和命令列交互验收已补齐；剩配色、解释视图和消息措辞的目测项。见 [本机验收记录](pr/acceptance-m3.md)。查询创建工具（F-QRY-013）移到 M7 |
| M4 表设计器及其他对象设计器 | 完成 | `feat/m4-remainder` | 40、41、42、43 | 查询创建工具留在 M7。偏差见第 4 节。见 [验收记录](pr/acceptance-m4-remainder.md) |
| M5 插件体系 | 未开始 | | 03-plugin-api | |
| M6 导入导出 | 未开始 | | | |
| M7 模型和 ER 图 | 未开始 | | | 含查询创建工具，与 ER 图共用画布 |
| M8 数据传输与同步 | 未开始 | | | |
| M9 备份与自动运行 | 未开始 | | | |
| M10 BI | 未开始 | | | |
| M11 AI 助手 | 未开始 | | | |
| M12 同步（替代 Cloud） | 未开始 | | | |
| M13 HTTP 隧道 | 未开始 | | | |
| M14 其他数据库 | 未开始 | | db/ | 已实现的 MySQL/MariaDB、PostgreSQL、SQLite 各自一个 crate。接入一个新数据库：新建不依赖 GPUI 的 crate，实现 `Driver`（`info()`、`connect()`）；关系型再实现 `Relational`（八个目录/补全方法用 `relational_catalog!` 转发，漏实现是编译错误）和 `Dialect`（词法、引用、`features()` 与少量返回 SQL 的方法）；最后在 `navidog-session::init` 注册具体驱动。界面只读能力方法与 `DialectFeatures`，不按产品名分支（`scripts/check-dialect-branches.sh` 检查） |
| M15 发布 | 部分 | `codex/macos-adhoc-signing` | 150-macos-distribution | 工作流、安装包、`latest.json`、应用内更新已在仓库里；无证书时免费临时签名与验签已接入并经本机验证。正式公证仍需证书与凭据（第 4 节），配置迁移（F-REL-004）未做 |

## 2. 当前里程碑：M4（表设计器）

已实现的三种数据库拆成独立 crate：`navidog-mysql`（含 MariaDB）、`navidog-postgres`、`navidog-sqlite`。`Dialect` 是 trait，扫描器只看 `Lex`。目录和补全 SQL 在各会话实现里；`navidog-db` 保留 `Driver`/`Session`/`Relational` 契约、连接与 SSH 编排、执行恢复和结果集读取。表设计的语句差异是方言上的能力方法，执行顺序仍在 `navidog-schema`。连接表单读 `DriverInfo` 的多主机、超时、连接参数和 SSL 模式。界面按方言能力方法（`like_text_cast`、`standalone_indexes`、`table_option_style`、`explain_style` 等）和 `DriverInfo` 标志分支，没有按 `Dialect::id()` 的比较。相对拆分前有三处行为变化：没有方言时查询补全为空（以前回退到 MySQL 关键字）；未绑定连接的网格按输入的原文显示，不再加引号；表设计器标签在连接消失后显示缺失连接面板，未保存的草稿关闭前要确认。验证见第 5 节。

### M3 收尾记录

M2 的任务都已完成，逐项记录见 git 历史；M2 的功能状态仍在第 3 节。

| 任务 | 状态 | 说明 |
| --- | --- | --- |
| SQL 编辑器技术验证（30-query-editor 第 1 节） | 完成 | 继续用 `EditorState`；括号高亮、查找、占位符用区间装饰；5 万行上色后滚动和输入平均约 2.9ms，数据见规格第 1 节 |
| 驱动扩展：列来源、会话 ID、PostgreSQL NOTICE、MySQL 读取超时、PostgreSQL 取消走 TLS | 完成 | 31 第 1 节。所有发出的 SQL 经 `navidog_db::log` 记录；`list_databases` 改为默认方法走 `execute` |
| 分句器返回区间并支持 `DELIMITER` | 完成 | `split_script` 按方言处理引号、`#` 注释、`$tag$` 和 E 字符串 |
| `navidog-query`：查询选项卡、目标选择、运行、停止、查询参数 | 完成 | 30 第 2 节，31 第 2、3 节 |
| 结果：结果选项卡、消息、摘要、固定、位置、命名、可编辑结果、保留之前的查询结果 | 完成 | 提交流程提成 `navidog_grid::commit`；结果有单元格编辑器和只读表单视图。保留结果时标签有关闭按钮；右键动作挂在查询编辑器上 |
| 查询分析和状态 [My] | 完成 | 31 第 4.5 节 |
| 查询解释：网格、文本、JSON、可视化、统计信息 | 完成 | 计划解析、占比、统计和布局在 `navidog_util::explain` |
| 编辑：查找替换、格式菜单、美化 SQL、简化 SQL、剪贴板堆栈、缩放、语法配色实测 | 部分 | 功能都已完成；语法配色与 Navicat 的并排实测未做，偏好设置可自定义 |
| 代码补全与代码补全信息 | 完成 | `navidog_db::outline`，关键字和内置函数在 `navidog-db` 的方言里 |
| 代码段，信息窗格的预览、代码段、标识符页 | 完成 | 存储在 `navidog_app_state::snippets` |
| 保存查询、对象分类“查询”、外部文件 | 完成 | `navidog_model::queries` |
| 自动恢复 | 完成 | 正常退出不询问，随布局恢复（30 第 8 节） |
| 偏好设置：编辑器、代码补全、自动恢复、高级页的日志选项 | 完成 | 30 第 9 节，31 第 6 节 |
| 历史日志、QueryExec.log | 完成 | 31 第 6 节 |
| 命令列界面 | 完成 | 31 第 7 节 |
| SQL 输入处的自动补全 | 完成 | 筛选文本、命令列、代码段编辑框和表设计器里写 SQL 的框共用 `navidog-sql` 的排序与命中高亮；见 [验收记录](pr/acceptance-sql-field-completion.md) |
| 批量删除后底部 SQL 状态文字溢出修复 | 完成 | 共享单行组件处理显式换行并保留悬停原文，表查看器与查询结果固定底栏高度；见 [验收记录](pr/acceptance-status-overflow.md) |
| TLS 依赖审计补丁更新 | 完成 | `rustls-webpki` 升至 0.103.16、`webpki-roots` 升至 1.0.10；audit 与 deny 均通过，未新增忽略项；见 [验收记录](pr/acceptance-status-overflow.md) |
| 查询执行后的对象与表格联动刷新 | 完成 | `codex/query-auto-refresh`；同连接已加载目录、DDL 和安全表格自动更新，编辑/事务保留，关闭查询窗口后仍通知；226 项测试、39 张本地截图及三项性能检查通过，见 [验收记录](pr/acceptance-query-refresh.md) |
| 联动刷新验证时的依赖审计补丁更新 | 完成 | `ctutils` 0.4.4、`tokio` 1.53.3 修复新公布的问题；audit 与 deny 通过，未新增忽略项，见同份验收记录 |

### 架构基础优化（独立分支）

从 `upstream/main` 的 `4c403de` 创建 `codex/architecture-foundations`，规格见 [05-architecture-foundations.md](design/05-architecture-foundations.md)。初版基线未含 M4；2026-10-24 冲突同步已合入最新 main 的 M4 与查询执行联动刷新。

| 任务 | 状态 | 验证与说明 |
| --- | --- | --- |
| 数据库执行与会话安全 | 完成 | DB live 15 项与 lib 内的三驱动操作会话回归：普通 SQL 不重放、元数据恢复、事务丢失守卫、重连后取消；生成写入使用独立短期会话，错误保留原始诊断及结构化状态 |
| 配置保存事务 | 完成 | core 失败注入与 connection UI：候选先落盘，失败不改变内存/密钥或重连；缺失 ID 在回调前整体拒绝，密码保存失败仍以本次凭据连接；StoreError 由 core 统一本地化 |
| 查询结果的数据与来源 | 完成 | query/model/viewer 回归：网格唯一拥有当前行，保留空白新增行；固定结果绑定原来源，识别的 USE/search_path 记录实际上下文及完整 PG 路径，来源未知时拒绝跨会话同步；脏排序拒绝，隐藏列拒绝回传，提交成功后的重载失败不重复写入。当前没有查询结果手动刷新用户入口，内部刷新守卫只是未来调用的约束 |
| PR #14 review 修复收尾 | 完成 | 八条评论均补充实现及回归；2026-10-22 基于最新 main 整理到 `codex/pr14-review-followup`，保留作者修复与 #15，实库模式 232 项/37 套件、静态检查、在线审计、37 张截图及三项 release 性能复验通过；显式重开保留本地草稿，恢复前提示旧事务状态和未知写入结果 |
| PR #17 AI 与作者 review 修正 | 完成 | 旧配置错误格式化入口与 DbError 结构体兼容、操作会话随关闭/重开取消、提交后保留独立空白草稿；手动刷新声明收窄为内部守卫与提交后同步，未增加用户入口。五条 AI 与七条作者评论已处理；249 项/39 套件、静态检查、在线审计、37 张截图与三项 release 复验通过，ctutils/Tokio 安全补丁已更新。逐条说明见 [验收记录](pr/acceptance-architecture-foundations.md#pr-17-ai-与作者-review-修正2026-10-24) |
| PR #17 与最新 main 冲突同步 | 完成 | 合并 `4ffa185` 的 M4 与查询执行联动刷新，保留操作生命周期和草稿保护，补齐自动刷新代际校验与排队重开回归；284 项/43 套件、fmt/clippy/裸值/审计、58 张浅深色截图及三项 release 复验通过。保留 main 依赖并升级 `crypto-bigint` 0.7.6，见架构验收记录 |

初版及 CI 时序修复在 2026-10-21 验证过 215 项测试与 29 张截图；两项基线证书依赖漏洞已补丁更新，deny 与在线 audit 当时通过。PR #14 的 Intel 代码段对话框测试已固定动画/调度时序，并连续复跑 20 次。本轮 review 修复的行为、逐条评论映射、回归入口及最终验证见 [验收记录](pr/acceptance-architecture-foundations.md)。

### 整体审查遗留工作（`refactor/remaining-work`）

来源：[2026-10-26 遗留清单](cr/2026-10-26-remaining-work.md)。该清单建议按主题拆成多个 PR；本次按用户要求放在同一个分支/PR，逐项纯迁移。验收记录见 [pr/acceptance-remaining-work.md](pr/acceptance-remaining-work.md)。

| 任务 | 状态 | 验证与说明 |
| --- | --- | --- |
| R-04 / D-05 统一 SQL 扫描与词法 | 完成 | 新 `navidog-sql` 的 `span::spans` 是唯一词法层（字符串/引号标识符/注释/`$tag$`/`[$name]` 参数/括号深度/语句边界，含 MySQL `DELIMITER` 与 `BEGIN…END`）；分句、参数、格式菜单、表格筛选、表设计解析共用它，各消费者的跳过函数已删。回归在 `navidog-sql` 的 span 测试与 `navidog-grid` 的跨消费者一致性测试 |
| D-08 / A-06 Catalog/OutlineCache 统一缓存 | 完成 | `ScopeCache<K,T>` 泛型化；Catalog 五类缓存用 typed key + `LoadId`，`require/require_all/reload_all/require_selection` 是唯一加载入口，objects 面板的 `ensure_loaded` 与 `stalled_load` 已删。目录生命周期、失败重试、延迟代际测试在 `catalog.rs`/`catalog_lifecycle.rs`/`database_refresh.rs` |
| A-01 拆 app-state | 完成 | `navidog-core` 删除，GPUI 状态迁到 `navidog-app-state`，纯数据留在 `navidog-model`；322 处 import 按模块迁移，无兼容 facade |
| A-02 core/驱动去 gpui | 完成 | `navidog-db` 只依赖 model；`navidog-headless` 用 model+db 跑 SQLite；`scripts/check-deps.sh` 在 CI 检查依赖方向并禁止 `crates/navidog-core` 回潮 |
| A-03 table_design 拆出 | 完成 | 新 `navidog-schema` 持有表草稿/表单/迁移计划，复用 R-04 的词法层；`rg 'table_design' crates/navidog-db/src` 为空，实库 DDL 测试随模块迁移 |
| A-07 god object/超长文件 | 完成 | viewer/results/editor/form/model/update 主文件分别为 768/348/922/534/139/27 行，全部 < 1000（update < 600）；`TableSession`/`TableView`、`ResultSession`、storage/edit/projection、`enum AfterCommit` 均已拆出；纯搬家，断言未改 |
| A-10 Connection `profiles + active` | 完成 | `profiles: Vec<Profile>` + `active` 取代字段交换；`connections.json` 写旧布局、读两种布局；7+ 配置文件/重名/空名/两种 JSON/panic 恢复都有测试 |
| S-07 共享 i18n 后端 | 完成（方案 1） | 保持 crate-local 契约；动态 key 必须登记 `scripts/i18n-dynamic-keys.txt`（19 条），`check-i18n.sh` 检查；契约写入 00-overview |

### M4 表设计器

| 任务 | 状态 | 说明 |
| --- | --- | --- |
| 规格与开发分支 | 完成 | 从 `codex/m3-completion` 的 `4bb20ea` 创建 `codex/m4-table-designer`；规格先单独提交 |
| `navidog-designer` 与导航入口 | 完成 | 对象工具栏新建/设计、表右键和表下元素入口；独立选项卡、关闭守卫、草稿随布局恢复；作者 review 已补对象身份、设计/数据分别复用、查看器菜单与快捷键 |
| 字段、约束、索引、选项与 SQL 预览 | 部分 | 八列字段网格、类型/长度/默认值等属性控件、结构化索引/外键/检查已完成；表达式和高级 SQL 框已接补全。三种方言选项表单和迁移已接入，存储引擎私有属性仍经高级定义编辑 |
| 迁移计划与保存 | 完成 | 三种方言及 MariaDB 实库验证；SQLite 重建先把旧表改名为 `_表_old_日期` 并留下，不删除；预览页去掉保存、改为执行；外键校验/依赖检查；外部结构变化拒绝覆盖；失败保留草稿 |
| 界面与本机检查 | 完成 | headless 编辑/预览/执行/失败恢复、条目重载重排、关闭守卫与无改动关闭；浅深色截图与第 5 节检查，详见验收记录 |
| 其余对象、安全与工具 | 完成 | 共用定义设计器、表设计器排除/触发器/分区、整表重建、用户与权限、维护、信息窗格依赖与成员、服务器监控、在库中查找。SQLite 用户认证扩展未编入发行版，面板只说明原因。见 [验收记录](pr/acceptance-m4-remainder.md) |
| 依赖审计补丁 | 完成 | `ctutils` 升至 0.4.5（RUSTSEC-2026-0385）、`tokio` 升至 1.54.1（RUSTSEC-2026-0386）；audit 与 deny 均通过，未新增忽略项 |
| 界面对齐 | 完成 | 表设计器编辑字号、连续关闭未保存选项卡、测试连接底栏、选中色、对象列表详细信息、系统对象开关、新建连接菜单、缺省主题与行底纹、关于文案、全局解锁；CI lint 修复后全工作区 clippy 与 334 项测试通过。见 [验收记录](pr/acceptance-ui-polish.md) |

## 3. 功能状态

只列出已经开始的里程碑涉及的功能；其余功能都处于“未开始”状态。

### M1

| 功能 | 状态 | 验证 | 说明 |
| --- | --- | --- | --- |
| F-APP-001 主窗口布局 | 完成 | snapshot、目测 | |
| F-APP-002 主工具栏 | 部分 | snapshot | 按钮尺寸是暂定值；除“连接”外的按钮在后续里程碑启用 |
| F-APP-003 菜单栏 | 部分 | 目测 | 菜单随对象窗格的当前选项卡切换：查询编辑器有查询和格式，表查看器有表，其余没有这两项。未实现的菜单项仍随各里程碑加入 |
| F-APP-004 状态栏 | 完成 | snapshot | |
| F-APP-005 编辑器选项卡 | 部分 | snapshot | M2 加入表查看器和 DDL 选项卡；弹出为新窗口未做 |
| F-APP-006 显示或隐藏窗格 | 完成 | 目测 | |
| F-APP-007 窗口与布局持久化 | 完成 | state 单元测试、`present_main_window_reopens_after_the_last_window_closes`、`present_main_window_targets_the_main_window_next_to_other_windows`、目测 | 关掉最后一个窗口后，点程序坞图标或「显示主窗口」按已保存布局再开主窗口；主窗口仍开着时只激活它，即使别的窗口在最前面，也不会另开一个。菜单命令在窗口自身 update 内派发，激活推迟到该 update 结束后 |
| F-APP-008 全屏与专注模式 | 部分 | | 全屏已完成；专注模式在 M2 |
| F-APP-009 浅色与深色外观 | 完成 | snapshot、theme 单元测试 | |
| F-APP-010 界面语言 | 完成 | settings 单元测试、目测 | |
| F-APP-017 日志文件 | 部分 | | 应用日志已完成；SQL 日志在 M3 |
| F-APP-018 关于与检查更新 | 完成 | update 单元测试、目测 | 两个渠道都读 nightly 上的 `latest.json`（没有则试最新稳定版；`releases` 是各版本列表），不含预发布时跳过预发布和 nightly。摘要为空则拒绝安装。下载校验 SHA-256，对话框显示下载进度，下载中可取消。nightly 用清单里的 `built_at` 和本构建时间严格比较，相同则已是最新。安装开始后改为不定进度，文案是挂载、复制、验证签名、替换或打开安装包。没有本机安装包时按钮是打开发布页，清单读取失败记 warn。启动检查最多每两天一次，失败不记检查时间。手动检查：没有活动窗口时用最前面的窗口；还没有发布版本单独提示。就地替换先把新包放到旁边，失败时删掉半成品再移回旧包。重新启动等当前进程退出后再打开。关于页：nightly 显示渠道和构建时间；tag 构建在编译前把 workspace 版本改成 tag |
| F-APP-019 欢迎界面 | 部分 | snapshot | 有空状态提示；首次启动引导窗口未做 |
| F-PREF-001 偏好设置窗口 | 完成 | 目测 | |
| F-PREF-002 常规 | 完成 | settings 单元测试、theme 单元测试 | 主题（Navidog、Default，缺省 Default）、外观、语言、自动连接、自动检查更新、包含预发布版本、显示模式下的对象、显示表下的对象、显示系统对象（缺省开，与查看菜单「显示隐藏项目」同一项）。已经写进 settings.json 的值不会被改掉 |
| F-PREF-010 高级 | 部分 | | 设置位置、主密码已完成；日志选项未做 |
| F-CONN-001 新建连接：选择类型 | 完成 | snapshot | |
| F-CONN-002 连接表单：常规 | 完成 | ui 测试、snapshot | |
| F-CONN-003 连接表单：高级 | 部分 | snapshot | 读取和写入超时只保存，到 M3 执行查询时才生效 |
| F-CONN-004 连接表单：数据库 | 完成 | 目测 | |
| F-CONN-005 连接表单：SSL | 完成 | live 测试（PostgreSQL） | 不支持 CRL（native-tls 的限制） |
| F-CONN-006 连接表单：SSH | 完成 | live 测试、代码检查 | 默认按 known_hosts 校验（首次信任），连接表单可关闭 |
| F-CONN-008 兼容性 [My] | 未开始 | | 按钮还没有接入 |
| F-CONN-009 备注 | 完成 | 目测 | |
| F-CONN-010 测试连接 | 完成 | live 测试、connection `a_locked_save_asks_for_the_master_password` | 结果在按钮行上方的通栏：测试中可取消，成功显示服务器版本，失败显示原始错误。见 [验收记录](pr/acceptance-ui-polish.md) |
| F-CONN-011 编辑、复制、删除连接 | 完成 | store 单元测试 | |
| F-CONN-012 打开、关闭连接 | 完成 | ui 测试、live 测试、tabs 测试、connection `reopen` 与 navigator `recovery_tests` | 关闭连接时关掉该连接的编辑器选项卡（含弹出窗口），不另询问；显式重新打开先确认，复用 Reopening 保留选项卡、查询文本和本地行草稿，旧事务与临时状态无法恢复。查看器与查询编辑器的草稿保留已由真实重开回归验证 |
| F-CONN-013 连接组 | 完成 | store 单元测试、navigator UI 测试 | M2 加入拖放和原地新建、重命名 |
| F-CONN-014 连接颜色 | 完成 | snapshot | |
| F-CONN-020 密码存储 | 完成 | crypto、store 单元测试、unlock UI 测试、`failed_password_save_still_uses_credentials_once`、`a_locked_save_asks_for_the_master_password` | 设计见 12-secrets.md。打开、编辑、保存、复制和修改主密码在锁定时都走同一个解锁对话框。macOS 调试构建一次读取仍可能连弹两次钥匙串框。重加密要么全部完成要么文件不动；保存密码失败提示未保存，本次连接仍使用内存凭据 |
| F-NAV-001 连接树 | 完成 | ui 测试 | M1 到数据库层级，M2 到对象层级（见下表） |
| F-NAV-003 筛选 | 部分 | | 搜索已完成；快速筛选在 M2 |
| F-NAV-006 右键菜单 | 部分 | | 连接和组的菜单已完成 |
| F-NAV-008 键盘操作 | 部分 | | 方向键、⌘R 已完成；Delete 删除未做 |
| F-INFO-001 常规 | 部分 | snapshot | 没有“共享”按钮（M2） |
| F-INFO-010 最大化与还原 | 完成 | 目测 | |

### M2

| 功能 | 状态 | 验证 | 说明 |
| --- | --- | --- | --- |
| F-APP-005 编辑器选项卡 | 完成 | tabs 测试、snapshot | 对象选项卡拖走会回到最左；选项卡条滚动时它仍会跟着走（第 4 节）。偏好设置的“在新窗口中打开”见 F-PREF-003 |
| F-APP-008 全屏与专注模式 | 完成 | 目测 | 表查看器工具栏右侧和“显示”菜单切换；隐藏导航窗格和信息窗格并进入全屏 |
| F-APP-011 收藏夹 | 完成 | favorites 单元测试、tabs 测试 | 收藏夹菜单、带 ID 的收藏（⌥⇧0~9 添加、⌃⇧0~9 打开） |
| F-APP-012 启动画面 | 完成 | tabs 测试 | 选项卡右键“添加到启动画面”，偏好设置“选项卡”页管理 |
| F-APP-013 最近使用 | 完成 | tabs 测试 | “文件 > 打开最近使用”，最多 10 项 |
| F-APP-014 通过 URI 打开 | 完成 | uri 单元测试 | 按连接名称匹配，其次按主机和端口；没有“其他选项”里的手动映射 |
| F-APP-015 通过 URI 共享 | 完成 | uri 单元测试 | 信息窗格“常规”页的“共享”按钮，可隐藏主机信息 |
| F-PREF-003 选项卡 | 完成 | 目测 | 在新窗口中打开、启动画面列表 |
| F-PREF-005 记录 | 完成 | settings 单元测试、model 单元测试、目测 | 限制记录、每页记录数、自动开始事务、修改后同步、点击编辑、网格字号、行底纹（缺省无）、无主键提示。字体系族仍是默认字体；数据分析颜色在 M10（第 4 节） |
| F-CONN-015 管理连接窗口 | 完成 | ui 测试 | 搜索匹配名称、类型、主机、端口、用户名、分组并高亮。工具栏直接调用连接命令。表格超出窗口时显示横向滚动条 |
| F-CONN-016 通过 URI 新建连接 | 完成 | uri 单元测试 | 支持 mysql://、mariadb://、postgres(ql):// 和带 `type` 的 navidog URI |
| F-CONN-019 重载 [My] | 完成 | reload 单元测试、MySQL 8.4 / MariaDB 11 实测 | 只提供两者都支持的 FLUSH 选项，见第 4 节 |
| F-CONN-021 连接配置文件 | 完成 | connection、store 单元测试、ui 测试 | 右键“切换连接配置文件”；PostgreSQL 多节点在常规页，live 测试覆盖。切换时编辑器选项卡保留并重新连上 |
| F-NAV-001 连接树（到对象层级） | 完成 | navigator 单元测试、database_refresh UI 测试、snapshot | PostgreSQL 多一层模式。打开连接后已展开的节点会重新加载；查询完成后刷新同连接已加载目录及打开的 DDL，旧请求不覆盖新缓存。目录在渲染中被读取时，树比较修订号后重绘。单击或方向键选到别的对象时回到“对象”选项卡 |
| F-NAV-002 表下元素 | 完成 | sqlite 测试、live 测试 | 字段、索引、外键、触发器。主键字段用钥匙图标。字段可右键重命名，双击打开表设计器并选中该字段 |
| F-NAV-003 筛选 | 完成 | navigator UI 测试 | 搜索与快速筛选（连接类型、活跃对象、已标星） |
| F-NAV-004 仅显示活跃对象 | 完成 | 目测 | 已打开的连接和已展开的数据库 |
| F-NAV-005 扁平连接 | 完成 | 目测 | 即“隐藏连接组” |
| F-NAV-006 右键菜单 | 完成 | 目测 | 后续里程碑的菜单项显示为禁用 |
| F-NAV-007 虚拟组 | 完成 | vgroups 单元测试 | 对象的虚拟组在连接目录的 `.vgroups.json`；连接的分组就是连接组 |
| F-NAV-008 键盘操作 | 完成 | navigator keys 测试 | M2 加入 Delete / ⌘⌫ 删除。方向键能走过表下的字段和文件夹行 |
| F-OBJ-001 对象列表视图 | 完成 | snapshot | 连接打开把列表清掉后，下一帧会重新请求 |
| F-OBJ-002 详细信息视图 | 完成 | 目测 | 可按列排序、选择列 |
| F-OBJ-004 对象工具栏 | 部分 | snapshot | 打开、删除、清空、截断已完成；设计、新建在 M4 |
| F-OBJ-005 对象组与扁平列表 | 完成 | 目测 | |
| F-OBJ-006 显示隐藏项和已删除项 | 部分 | live 测试 | 显示隐藏项目（系统库和系统模式）已完成；“已删除的项目”属于 Navicat Cloud，不实现 |
| F-OBJ-007 表概况 | 完成 | objects 单元测试 | “查看 > 显示表概况”。列表底部显示当前选中对象已有的属性，不另查目录 |
| F-OBJ-008 对象操作 | 部分 | dialect 单元测试、sqlite 测试、live 测试、DB `sessions::tests` | 删除、清空、截断、复制表、重命名已完成，生成 SQL 使用独立短期会话；断连错误提供原连接的重新打开入口。新建、设计在 M4 |
| F-INFO-001 常规 | 完成 | snapshot | 含“共享”按钮；有多个配置文件时显示当前配置文件 |
| F-INFO-002 DDL | 完成 | live 测试、snapshot | |
| F-INFO-007 字段 | 完成 | info 单元测试 | 信息窗格字段页在表查看器选中列时只显示该列，没有对应列时显示全部 |
| F-DATA-001 网格视图 | 完成 | grid UI 测试、spike、snapshot | 表头排序图标与基线不同（见第 4 节） |
| F-DATA-002 表单视图 | 完成 | 目测 | 当前页逐字段编辑，上一条/下一条。单元格编辑器见 F-DATA-011 |
| F-DATA-011 单元格编辑器 | 完成 | editor 单元测试 | 网格下方的文本、十六进制、图像、网页、JSON、枚举/集合。JSON 页有语法高亮。二进制图像只预览；网页只显示源码；枚举值只来自类型字符串 |
| F-DATA-012 外键数据选择 | 完成 | metadata 单元测试、sqlite 测试 | 右键打开被引用列的值。同一列多条外键用第一条；单击写入；默认 1000 条 |
| F-DATA-003 编辑记录 | 完成 | grid UI 测试、viewer UI 测试、date 单元测试 | 离开已修改的行会提交；翻页、排序、筛选、刷新前也先提交。日期、时间、日期时间列双击打开选择器 |
| F-DATA-010 原始数据模式 | 完成 | dialect 单元测试、model 单元测试 | 字符串列显示为带引号的 SQL 字面量；之后输入的非空文本按表达式写入 |
| F-DATA-004 事务 | 完成 | live 测试、viewer UI 测试 | 偏好设置可在打开表时自动开始事务。事务里每批修改用保存点包住 |
| F-DATA-005 区域选择与复制粘贴 | 完成 | model 单元测试、grid UI 测试 | 表头点击或菜单可选中整列。拖动表头经过单元格不改选区。焦点不在网格时，右键会先聚焦再弹出菜单，菜单项才能生效。表头右键在捕获阶段停掉 DataTable 的空浮层，避免它抢走焦点 |
| F-DATA-006 筛选与排序窗格 | 完成 | filter 单元测试、`where_text_collapses_a_pasted_line_break`、`a_single_line_sql_editor_does_not_scroll` | 创建工具、文本模式、右键快速筛选。文本模式的 WHERE 和 ORDER BY 用单行 SQL 编辑器补全当前表字段、条件关键字（`AND`/`OR` 等，不含 `SELECT`）和已载入的表。括号只包住全部条件；在列表的值不含逗号 |
| F-DATA-007 查找与替换 | 部分 | model 单元测试 | 查找栏是模式下拉（查找 / 替换 / 查找列）加输入框、上一个、下一个、完成。查找框回车是下一个，替换框回车是替换当前并跳到下一个。递增搜索在模式菜单里，默认关。不跨页 |
| F-DATA-008 导航栏与分页 | 完成 | live 测试、viewer ownership 测试、query_refresh UI 测试、snapshot | 刷新按钮在加载时旋转，至少半圈；外部查询后自动更新当前页、总数及字段，保留筛选排序分页；编辑和事务阻止自动刷新并提示 |
| F-DATA-009 格式设置 | 完成 | model 单元测试、dialect 单元测试、grid UI 测试、column_widths 单元测试、viewer 投影回归 | 隐藏列、行高、冻结到此列、冻结列、解除所有冻结列、恢复默认顺序、拖拽列顺序、数字右对齐。隐藏列投影拒绝时保留菜单、列与待提交值；成功后才更新菜单。打开表按内容定列宽；拖过的列按表保存在连接目录的 `.column-widths.json`，只记下拖动过的列。列顺序和冻结只在打开的查看器里保留 |
| F-DATA-013 字段信息 | 完成 | grid 单元测试、目测 | 同一个表头单元格里显示字段名和带颜色的类型图标；右键可关。显示类型时表头 34pt，数据行保持设置的行高 |
| F-DATA-014 显示 SQL | 完成 | truncated UI 测试、snapshot、目测 | 底部栏显示最近的 SQL；批量 SQL 的换行折成空格，单行省略，悬停保留全文。预览窗口用只读 SQL 编辑器，含待提交语句；见 [验收记录](pr/acceptance-status-overflow.md) |

### M3

| 功能 | 状态 | 验证 | 说明 |
| --- | --- | --- | --- |
| F-QRY-001 查询编辑器 | 完成 | query editor 测试、snapshot | 行号、折叠、括号高亮、自动换行、缩放；编辑器偏好即时生效。不自动补右括号 |
| F-QRY-002 代码补全 | 完成 | completion 单元测试（含 `a_completed_word_gains_a_space_and_a_snippet_keeps_its_text`、`a_completion_replaces_the_whole_identifier_and_keeps_space_outside_quotes`）、sqlite 测试、live 测试、outline 单元测试、`a_column_name_uses_the_property_color` | `.` 之后按数据库、表、别名给出候选；前缀、连续包含、跨字符三档排序（`mocfg` → `mount_cfgs`），命中的字符涂蓝；替换整个标识符，词末尾加一个空格，已有引号不套第二层，代码段按原文插入。字段名记到主题的 `property` 色，子句片段里的标识符同样。Tab 确认、Esc 打开；补全信息在连接目录的 `.outline/`。排序在 `navidog-sql`，筛选文本、命令列、代码段编辑框和表设计器共用。补全浮层没有图标（偏差） |
| F-QRY-003 查找与替换 | 完成 | find 单元测试、query editor 测试 | 模式下拉里有正则表达式、大小写符合、整个字；递增搜索始终开着，到末尾不回绕。查找框回车是下一个，替换框回车是替换当前并跳到下一个 |
| F-QRY-004 SQL 格式 | 完成 | sql_format 单元测试、query editor 测试 | 缩排、注释、大小写、美化 SQL（sqlformat，记号序列不变）、简化 SQL，每条命令一步撤销 |
| F-QRY-005 剪贴板堆栈 | 完成 | format 单元测试、query editor 测试 | 10 项，⇧⌘V 连按依次替换 |
| F-QRY-006 运行 | 完成 | query editor 测试、query_refresh UI 测试、live 测试 | 运行、运行当前语句、运行已选择的、停止、遇到错误时继续；出错的语句用诊断标出。执行结束通知原连接更新，包含部分失败、取消和执行中关闭窗口；解释不通知，固定结果保留执行时快照。重连后重放最近一次成功的 `USE` / `SET search_path`。⌘R 在查询编辑器和表查看器里不落到全局刷新 |
| F-QRY-007 查询结果 | 完成 | query editor 测试、results 单元测试、truncated UI 测试、snapshot、`accepted_results_keep_their_rows_and_original_source` | 最多 20 个结果集；`-- NAME:` 命名；固定、三种位置。“保留之前的查询结果”默认关闭，打开后留下上次的结果和解释，标签上可关闭；消息和摘要仍只记这一次。结果标签右键在焦点停在编辑器时生效。已有结果时切到新页面仍显示结果；结果页上的位置按钮直接切换，不依赖焦点。查询结果右键没有快速筛选。单表且含主键的 SELECT 可编辑；待提交值或空白草稿阻止排序，保留已接受排序箭头；提交后同步按原结果来源读取，重载失败不重复写入。当前没有结果手动刷新用户入口，内部守卫不能视为已交付的交互。底部 SQL 和提交状态保持单行，悬停保留原文 |
| F-QRY-008 查询解释 | 完成 | explain 单元测试、live 测试（MySQL 8、MariaDB 11、PostgreSQL 16）、snapshot | 可视化、网格、文本、JSON、统计信息、信息；不能下载 PNG（偏差） |
| F-QRY-009 查询参数 | 完成 | query editor 测试、sql_text 单元测试 | |
| F-QRY-010 代码段 | 完成 | snippets 单元测试、info 测试、query editor 测试 | 预设片段只读；占位符 Tab 跳转 |
| F-QRY-011 保存查询 | 完成 | queries 单元测试、query editor 测试 | 无标题先问名称；外部文件；外部修改提示重新加载；对象分类“查询” |
| F-QRY-012 历史日志 | 完成 | log 单元测试、history 测试、snapshot | 仅显示错误、暂停、表头开关 |
| F-QRY-014 命令列界面 | 完成 | console 单元测试、console 测试、Unicode 列宽测试、`console_enter_confirms_completion_instead_of_running` | mysql / psql 的输出样式；输入区与查询编辑器同一套补全，菜单打开时回车不执行、方向键不翻历史。普通 SQL 完成后与查询面板共用刷新通知，关闭窗口不丢通知，未完成的输入不通知；不支持客户端命令（偏差） |
| F-INFO-004 预览 | 完成 | 目测 | |
| F-INFO-005 代码段 | 完成 | info 测试、`an_untyped_snippet_offers_no_names_and_a_typed_one_keeps_its_driver` | 标签筛选、搜索、右键菜单、拖到编辑器。编辑框按所选数据库类型补全关键字、内置函数和同类型片段名，画在主窗口遮罩上以免菜单被挡住；类型为“全部”时不补全 |
| F-INFO-006 标识符 | 完成 | info 测试 | 双击或拖动插入名称 |
| F-APP-016 自动恢复 | 完成 | recovery 单元测试、query editor 测试 | 正常退出不询问（偏差） |
| F-APP-017 日志文件（SQL 日志） | 完成 | log 单元测试 | QueryExec.log 每次启动清空 |
| F-PREF-004 编辑器 | 完成 | theme 单元测试（含 `syntax_colors_override_the_theme` 的字段色）、目测 | 语法颜色按外观分开保存，含字段色 |
| F-PREF-006 代码补全 | 完成 | 目测 | |
| F-PREF-008 自动恢复 | 完成 | 目测 | |
| F-PREF-010 高级（日志选项） | 完成 | 目测 | 应用日志的位置和级别下次启动生效 |
| F-CONN-003 连接表单：高级（读取超时生效） | 部分 | live 测试 | MySQL 读取超时生效；写入超时仍只保存，见 31 第 10 节 |

### M4

| 功能 | 状态 | 验证 | 说明 |
| --- | --- | --- | --- |
| F-DSG-001 表设计器：字段 | 部分 | designer headless 测试、snapshot、table_design 测试（含 `chosen_types_use_the_spelling_the_server_prints_back`）、`expression_completion_lists_draft_columns_and_definition_lists_types`、`an_unedited_definition_and_check_keep_their_line_breaks` | 八列字段网格与下方属性区；类型菜单使用该库回读的拼写，整数类型不编辑长度；长度/小数点/不是 null/主键/MySQL 注释及默认值、排序规则、字符集、无符号、零填充、ON UPDATE、自动递增、PG identity 模式和生成表达式控件。默认值、生成表达式、ON UPDATE 按草稿字段名和条件关键字补全，字段定义用整份关键字并另列类型名。拖放、完整方言属性和运行态尺寸对照未完成 |
| F-DSG-002 表设计器：索引 | 部分 | 三服务端与 SQLite 实库、snapshot | 结构化索引列表、字段选择、方法、唯一、注释/部分索引条件表单；MySQL KEY 同页编辑；复杂索引属性保留到高级页 |
| F-DSG-003 表设计器：外键 | 部分 | 三服务端实库、SQLite 级联 UI/外键校验/回滚测试、浅深色截图 | 独立外键列表及结构化表单；引用模式/表/字段从连接元数据级联选择，支持按序多字段、自引用草稿、异步过期隔离与失败重试；刷新图标至少转半圈；类型/唯一键兼容性由执行检查，完整方言属性待做 |
| F-DSG-004 唯一与检查 | 部分 | SQL 计划测试、三服务端与 SQLite 实库、`an_unedited_definition_and_check_keep_their_line_breaks` | 检查名称与表达式表单、索引唯一开关；MySQL 移除部分 CHECK 明确拒绝；唯一约束有单独定义并保留，完整方言属性待做 |
| F-DSG-007 表设计器：选项 | 部分 | 选项单元测试、三服务端与 SQLite 实库、designer UI、浅深色截图 | MySQL 23 项常用选项；PG 拥有者/表空间/继承/存储参数/UNLOGGED，含事务修改与并发检测；SQLite STRICT、WITHOUT ROWID（含已有表，预览写明隐式 rowid 会丢掉）。引擎私有属性经高级定义保留，未知参数变更明确拒绝 |
| F-DSG-009 注释与 SQL 预览 | 完成 | designer headless（含预览页放弃修改后立即清掉 SQL）、PG 实库、浅深色 snapshot、注释计划测试 | 表/字段注释与只读 SQL 预览；PG 支持已有注释读取、修改、清空、重命名跟随、复制、旧草稿迁移、失败回滚和外部变化检查；SQLite 无数据库注释语法 |
| F-DSG-010 表设计器：保存 | 完成 | table_design 实库、designer headless、tabs 测试、`postgres_column_reorder_rebuilds_without_cascade`、`postgres_rebuild_keeps_constraint_names_and_serial_defaults`、`postgres_identity_can_be_added_and_sequence_options_stay_fixed`、`sqlite_rebuild_keeps_generated_columns_and_combines_renames`、`sqlite_rename_with_a_trigger_edit_keeps_the_other_trigger`、`sqlite_rename_and_trigger_edit_keeps_update_of_and_the_view`、`sqlite_strict_rebuild_preserves_data_and_converts_without_rowid`、`live_type_names_round_trip_from_the_server` | 新建/差异迁移、先预览后执行、失败保留草稿、目录与补全刷新。PostgreSQL 现有列重排、identity 添加/删除/模式切换、SQLite 生成列、已有表 WITHOUT ROWID 和合并重命名走事务内整表重建。命名主键/唯一/排除先用临时名，原来就有的列上的 `nextval` 默认值先解除序列归属再挂回。新建列的 `nextval` 不改序列归属。序列的 start/increment 不能改。未知选项、删除仍被依赖的字段、附加库上的表重建仍拒绝 |
| F-DSG-005 排除约束 | 完成 | `exclude` SQL 计划测试 | PostgreSQL 排除约束可添加、修改、删除并预览 |
| F-DSG-006 触发器 | 完成 | 表设计计划测试 | 三种库可编辑触发器文本；SQLite 只改触发器时不重建整表。PostgreSQL/MySQL 打开已有表时读取触发器定义，列重排重建会按原文建回。表单仍是自由文本，事件固定为 `FOR EACH ROW` |
| F-DSG-008 分区 | 完成 | 表设计计划测试、`mysql_partition_keeps_the_partition_name` | MySQL 可改分区，填写的分区名写入 `PARTITION` 子句；PostgreSQL 已有表不能改分区方式，已有子分区不加载 |
| F-DSG-011 视图设计器 | 完成 | `definition_plans_cover_each_object_kind` | 定义、MySQL 算法/安全性/检查选项、SQL 预览、预览数据。已有视图仍按整段 DDL 打开。新建后再次保存时，外部变更守卫比较服务器 DDL，不拿编辑器正文去比。查询创建工具在 M7，面板里不出现 |
| F-DSG-012 实体化视图 | 完成 | 同上、`refresh_materialized_view` | 新建、SQL 预览、刷新、预览数据 |
| F-DSG-013 函数与过程 | 完成 | 同上 | 新建、SQL 预览、带参数时先询问再运行 |
| F-DSG-014 事件 | 完成 | 同上 | 名称、计划、定义；计划必须是一条语句 |
| F-DSG-015 数据库与模式 | 完成 | 同上 | 新建和编辑。已有库不能改字符集；删除数据库的菜单项仍禁用 |
| F-DSG-016 附加数据库 | 完成 | 同上 | 附加、分离、查看 sqlite_master。不能分离 main |
| F-DSG-017 表空间 | 完成 | 同上 | 新建；已有表空间不能改位置 |
| F-DSG-018 PostgreSQL 其他对象 | 完成 | 同上 | 类型、域、序列、扩展、外部服务器、聚合、转换、运算符、排序规则。计划测试覆盖每种语句 |
| F-DSG-019 MySQL 其他对象 | 完成 | 同上、`lists_kind` | MariaDB 10.3+ 显示序列；MySQL 不显示 |
| F-OBJ-009 维护 | 完成 | `security_statements_cover_each_product` | 表和数据库菜单打开维护面板，先显示语句再执行。SQLite 只对 main 做 VACUUM |
| F-INFO-003 使用与被使用 | 完成 | `relation_plan` | 信息窗格页。PostgreSQL 用 pg_depend；MySQL 只用外键目录；SQLite 在 sqlite_master 里按名字匹配 |
| F-INFO-008 成员、成员属于、权限 | 完成 | 同上 | 信息窗格页，只读。SQLite 显示缺少用户认证扩展，不执行 SQL |
| F-SEC-001 MySQL 用户 | 部分 | `security_statements_cover_each_product` | 工具栏打开用户列表；双击进入设计器（常规、高级、成员属于、成员、服务器权限、对象权限、SQL 预览）。密码不回读，预览里显示为 `***`。实库创建仍未跑 |
| F-SEC-002 MySQL 角色 | 完成 | 同上 | 角色走 CREATE ROLE，不带主机 |
| F-SEC-003 PostgreSQL 用户、角色、组 | 完成 | 同上 | 组是 NOLOGIN。可改登录、超级用户、建库、建角色、继承、复制、连接限制和有效期 |
| F-SEC-004 权限管理员 | 完成 | 同上 | 以对象为中心授予或撤销。SQLite 不提供，面板说明原因 |
| F-SEC-005 SQLite 用户 | 部分 | 同上 | 发行版没有用户认证扩展。面板说明原因，不执行 SQL |
| F-TOOL-001 服务器监控 | 完成 | 同上 | 进程、变量、状态，当前页有选中态，自动刷新走当前页。PostgreSQL 结束进程用 `pg_cancel_backend`，变量页只读。移到新窗口不停止刷新。SQLite 不提供 |
| F-TOOL-002 在库中查找 | 完成 | 同上 | ⇧⌘F。包含、全字、前缀、正则和区分大小写。非法正则在查找时报告，不发给服务器。SQLite 正则在计划阶段拒绝；MySQL 区分大小写的 LIKE 用 `LIKE BINARY`，正则在模式前加 `(?-i)`。结果仍是文本，不能双击。数据搜索每个对象最多 100 行 |

### M15

| 功能 | 状态 | 验证 | 说明 |
| --- | --- | --- | --- |
| F-REL-001 打包 | 部分 | publish 工作流（`main` 推送与每日定时；安装包经 artifact 汇总后统一替换 `nightly`）；PR 预览构建产出 macOS aarch64 DMG artifact（7 天）；`scripts/check-macos-bundle-icon.sh` | macOS DMG、Windows zip / MSI / `.msi.zip`、Linux deb / rpm / AppImage / tarball，附 `latest.json` 和 `SHA256SUMS`。`cargo bundle` 的图标路径相对仓库根目录，打包后检查 `navidog.icns`。未在每台机器上安装验证 |
| F-REL-002 签名与公证 | 部分 | 本机真实签名/篡改/DMG 回归、完整应用首帧；publish/lint 配置检查 | 无证书时本地打包和发布共用免费临时签名脚本，严格验签失败则停止；完整签名应用经过 DMG 复制、整包替换仍可启动。两架构 CI 已接入回归，远端未跑；正式签名与公证未实测。见 [验收记录](pr/acceptance-macos-signing.md) |
| F-REL-003 自动更新 | 完成 | update 单元测试 | 与 F-APP-018 同一条路径：读 `latest.json` 的 `releases` 并校验 SHA-256。tag 的发布说明由 changelogithub 写入清单。tag 在清单上传成功后取消 draft。tag 构建在编译前把 workspace 版本改成 tag，不改仓库里的 Cargo.toml |
| F-REL-004 迁移或升级 | 未开始 | | 配置目录还没有版本迁移 |

## 4. 临时偏差与遗留

| 项目 | 引入于 | 计划处理 |
| --- | --- | --- |
| 表设计器已恢复字段网格和结构化表单；引擎私有属性表单、字段拖放和运行态 Navicat 并排仍未做 | M4 | ⌘↑ / ⌘↓ 移动字段。并排对照不阻塞本里程碑 |
| 未知表选项、删除仍被依赖的字段、附加库上的表重建仍然拒绝 | M4 | 不自动丢掉依赖对象，不猜测生成表达式里的标识符 |
| 不做加密 SQLite，不换 `bundled`，不做就地加密或解密，不做 SQLite 2 | M4 | 保持 rusqlite `bundled` |
| 发行版没有编入 SQLite 用户认证扩展 | M4 | F-SEC-005 保持部分：面板说明原因，不执行 SQL |
| 类型选择和连接表单是窗口内的模态对话框，而不是独立窗口 | M1 | AX 实测后决定 |
| 暂定 token：工具栏、导航树行、按钮高度、连接表单尺寸、偏好设置分类栏宽度 | M1 | 运行 `tools/ax-dump.swift` 后替换。偏好设置窗口已按 nib 改为 640 × 480 |
| SQLite 加密数据库、新建 SQLite 2 不支持 | M1 | 已决定不做。见上一行 |
| 结果网格每个可排序列的表头都显示默认排序图标，Navicat 只在已排序的列上显示 | M2 | 自绘表头，或向 gpui-component 提议选项 |
| 修改标记颜色（已修改单元格、待删除行）是暂定值 | M2 | 与 Navicat 并排实测后替换 |
| SQL 编辑器的语法配色未与 Navicat 并排实测 | M2 | 与 Navicat 并排实测后替换 `themes/navicat.json`；偏好设置“编辑器”页可以自定义 |
| 表查看器的总行数查询和数据查询共用一个会话，按顺序执行 | M2 | 大表 COUNT 拖慢翻页时另开会话 |
| 关闭有未提交修改的表查看器时，gpui-kit 的关闭按钮隐藏，标题里用 ● 代替：gpui-kit 没有关闭前询问的钩子，靠 `Panel::closable` 拒绝关闭再由 `navidog_ui::tabs` 的关闭守卫询问 | M2 | gpui-kit 提供关闭前钩子后改用它 |
| MySQL 的 UPDATE 不检查是否匹配到行：它报告的是实际改变的行数 | M2 | 需要时对会话打开 `CLIENT_FOUND_ROWS` |
| 关闭弹出的编辑器窗口时直接放回主窗口，Navicat 会询问 | M2 | 保证未提交的修改不丢失；需要询问时再加对话框 |
| 对象选项卡里虚拟组显示为“组标题 + 缩进的成员”，Navicat 是可进入的文件夹 | M2 | 与 Navicat 并排实测后调整 |
| SQLite 的外键没有名称，表下的外键显示为“→ 被引用表” | M2 | 无 |
| 截断文本的悬停提示在文本完整显示时也会出现 | M2 | 需要时测量排版宽度，只在截断时显示 |
| 滚动条在指针位于滚动区域时才出现（`ScrollbarMode::Hover`）。悬停才出现时，鼠标不能靠一条看不见的滚动条开始横向滚动，竖直滚轮也不会被转成横向。选项卡条没有独立滚动条，标题上的竖直滚轮会转成横向滚动 | M2 | 无 |
| 重载只提供 PRIVILEGES、LOGS、STATUS、TABLES、USER_RESOURCES：MySQL 8.4 已移除 FLUSH HOSTS，MariaDB 不支持 OPTIMIZER_COSTS；每个选项单独执行 | M2 | 按服务器版本显示可用选项 |
| PostgreSQL 多节点的成员用多行文本（每行一个 主机:端口）编辑，Navicat 是可增删的列表；经 SSH 隧道时只连第一个节点 | M2 | 与 Navicat 并排实测后调整 |
| 连接配置文件包含常规、高级、SSL、SSH 四页的设置；名称、分组、颜色、备注所有配置文件共用 | M2 | 无 |
| 通过 URI 打开时没有“其他选项”（手动映射到现有连接或新建连接）；匹配不到时提示错误 | M2 | 需要时加映射对话框 |
| 选项卡上的连接颜色画在标题下方的细条，Navicat 的确切样式待实测 | M2 | 与 Navicat 并排实测后调整 |
| 隐藏列是对已加载页的投影，不能隐藏最后一列；待提交的修改会先提交再重新投影 | M2 | 无 |
| 行高是对话框里输入的点数，只作用于当前网格 | M2 | 与 Navicat 并排实测后调整 |
| 表单视图只编辑当前页，用上一条/下一条切换记录 | M2 | 需要跨页记录导航时再接分页 |
| 原始数据模式在“查看”菜单和表查看器工具栏，不在“显示”子菜单；只影响之后输入的值 | M2 | 无 |
| 日期选择器按周一作为一周的开始；时间列只有文本框和确定，日期列点选即写入 | M2 | 需要时改成跟随系统周起始日；“今天”已按本地时区取日期 |
| 查找和替换只搜索当前已加载的页，不跨页 | M2 | 需要时按页继续查找 |
| 筛选括号只包住全部条件，不能嵌套；“在列表”按逗号拆开，值本身不能含逗号。解析不了的 WHERE 留在文本模式 | M2 | 需要多层括号时再把条件做成树 |
| 单元格编辑器在网格下方，不在信息窗格。十六进制是单元格文本的 UTF-8；网页只显示 HTML 源码；二进制图像只预览不写入；枚举和集合只在类型字符串列出取值时出现；JSON 页有高亮，没有语法高亮的语言菜单 | M2 | 网格能保存原始字节后再把图像写回单元格 |
| 外键选择只列出被引用列，不显示该表的其他列；同一列有多条外键时用第一条；单击写入，不是双击 | M2 | 需要整行预览时再加列清单 |
| 列顺序和冻结只留在打开的表查看器里，不写入连接目录。行标记列始终固定在最左。SQLite ROWID 是查询末尾的只读列，不写入 UPDATE/INSERT | M2 | 需要记住列顺序时再按表写入该连接的目录 |
| 双击列顶右边框把该列再调成适合内容（`data_format.html`）。`DataTable` 的拖动手柄在 gpui-kit 内部，delegate 挂不上 | M2 | gpui-kit 把双击列边交给 delegate 后再做 |
| 网格字体只改字号，字体系族仍是界面默认字体；没有系统字体面板 | M2 | 需要选字体时再接系统字体面板 |
| 关掉“修改后同步记录”后，未填写的新列在提交前显示 `(Default)`，生成值要手动刷新才出现；每三行底纹不涂表格下方的空白行 | M2 | 无 |
| 对象选项卡固定为第一个，拖走会弹回。选项卡条滚动时它仍一起移动：gpui-kit 的 TabBar 没有单个选项卡冻结 | M2 | 向 gpui-component 要选项卡条前缀，或自绘选项卡条 |
| 网格列线、新增行、选中选项卡和工具栏图标的颜色按一张深色截图暂定 | M2 | 与 Navicat 并排实测后替换 |
| macOS 调试构建读取本机密钥时，系统可能连续弹出两个钥匙串授权框。“始终允许”绑在代码签名上，每次编译签名都变 | M1 | 正式签名（M15）后授权才能留下来。读取仍用 `keyring`，见 [12-secrets.md](design/12-secrets.md) |
| 免费发布使用 ad-hoc 签名，完整性已验，但没有 Apple 开发者身份或公证，系统仍可能要求“仍要打开”。正式签名仍用 `codesign --deep`，只给 DMG staple，更新换上的 `.app` 没有票据 | M15 | 免费路径无需客户端重签，建议装到用户可写目录；公证需证书与 Apple 凭据同时存在。正式分发在有公证包后再改成逐个签名并给 `.app` staple；钥匙串授权变化仍按 12-secrets 的遗留处理 |
| 数据分析的条形图、高亮颜色不在记录偏好里 | M2 | M10 |
| 补全浮层没有图标，用说明文字区分对象类型；关掉“自动选择第一个建议项目”后第一项仍画成选中的样子 | M3 | gpui-kit 的补全浮层支持图标后改用 |
| 拖进编辑器的文字插在光标处，不在落点；不能从编辑器把代码拖进代码段页 | M3 | gpui-kit 公开坐标到文本位置的换算后改为落点 |
| 正常退出时不询问是否保存查询：未保存的文本写入自动保存，随布局回到原选项卡；“恢复”是主窗口里的对话框 | M3 | 需要时加退出询问 |
| 查询结果的修改在独立操作会话上提交，不参与编辑器里手动开启的事务；表单视图只读 | M3 | 共享元数据会话不再承接生成写入；查询编辑事务是否合并仍待 Navicat 并排实测后决定 |
| 代码补全的别名只看当前语句里的 FROM / JOIN，不解析子查询作用域 | M3 | 需要时接语法树 |
| 可视化解释不能下载为 PNG；节点尺寸暂定 | M3 | 需要时再做 |
| 命令列界面分输出区和输入区，不支持 `\d`、`.tables` 等客户端命令 | M3 | 需要时加显示宽度表 |
| 弹出窗口里的查询编辑器没有代码段和标识符页 | M3 | 无 |
| MySQL 的“写入超时”仍只保存 | M3 | mysql_async 提供发送超时后接上 |
| `navidog-schema` 的表选项解析（`mysql_span`/`pg_storage`/`pg_clause`）仍在通用 crate 里，只按 `TableOptionStyle` 分派；表设计在 `enriches_from_catalog` / `rebuilds_tables` 下仍直接执行 PostgreSQL catalog SQL 和 SQLite 重建 SQL（`sqlite_schema`、`sqlite_sequence`、`PRAGMA`）；关键字表是手抄的保留字集合，不是脚本从官方文件生成 | 拆 crate | 接第四个数据库前把选项解析和上述 SQL 搬进方言方法（会形成 schema → 方言 crate 的依赖，需要在 00-overview 里先定方向），并让关键字表由脚本生成 |
| 已有视图/函数/事件仍按整段 DDL 打开；用户面板不能改已有账号；查找结果不能双击；PostgreSQL 已有分区子表不加载；触发器表单没有事件和语句级；库/模式选项不标脏。列重排丢触发器、命名约束/`serial` 重建失败、二次保存被拒、移窗取消监控、分区名、监控语义和查找 SQL 已在 `9dccbb6` 修掉。其后补了冲突检查读失败、空基线禁止再保存、新建列不改序列归属，以及 MySQL 区分大小写正则 | M4 | 按 [pr/review-pr-31.md](pr/review-pr-31.md) 第 7 节的未修清单做。相关功能行仍标「完成」，不在本轮整批改成「部分」 |

## 5. 验证

整体审查修复的验收记录见 [pr/acceptance-overall-review.md](pr/acceptance-overall-review.md)。

| 检查 | 命令 | 说明 |
| --- | --- | --- |
| 格式与静态检查 | `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings` | CI 执行 |
| 裸值检查 | `scripts/check-raw-px.sh` | CI 执行 |
| i18n 检查 | `scripts/check-i18n.sh` | CI 执行；字面量 key 必须在本 crate 定义，动态 key 必须登记在 `scripts/i18n-dynamic-keys.txt` |
| 方言分支检查 | `scripts/check-dialect-branches.sh` | CI 执行；生产代码不得比较 `dialect.id()`，方言差异写成能力方法，界面能力写成 `DriverInfo` 标志 |
| 依赖方向检查 | `scripts/check-deps.sh` | CI 执行；model/db/sql/schema/util/sql-log/headless/mysql/postgres/sqlite 的 normal 依赖不得出现 `gpui`，`crates/navidog-core` 不得回潮 |
| 依赖审计 | `cargo deny --all-features check`、`cargo audit` | CI 的 security-audit 工作流执行，配置在根目录 `deny.toml` 和 `.cargo/audit.toml`；只有直接依赖的 unmaintained 会失败，无补丁的 `RUSTSEC-2023-0071`（rsa）在两边都忽略 |
| 单元测试与 headless UI 测试 | `cargo test --workspace` | CI 执行 |
| 实库测试 | `docker compose up -d --wait && NAVIDOG_LIVE=1 cargo test -p navidog-db -p navidog-schema -p navidog-query --lib --test live --locked` | CI 的 integration 工作流和本地都执行；`--lib` 包含操作写入完成后元数据断连恢复的三驱动回归和 `navidog-schema` 的表设计 DDL 回归，测完执行 `docker compose down` |
| 截图 | `cargo test -p navidog --test snapshot` | 本地执行，输出 `target/snapshots/` |
| 结果网格性能 | `cargo run --release -p navidog-grid --example spike` | 本地执行，需要 Metal |
| 导航树性能 | `cargo run --release -p navidog-navigator --example tree_spike` | 本地执行，需要 Metal |
| SQL 编辑器性能 | `cargo run --release -p navidog-query --example editor_spike` | 本地执行，需要 Metal；`NAVIDOG_SPIKE_LINES` 改行数，截图写到 `target/editor-spike-*.png` |
| 查询解释实库测试 | `docker compose up -d --wait && NAVIDOG_LIVE=1 cargo test -p navidog-query live_plans` | 本地执行；MySQL 8、MariaDB 11、PostgreSQL 16 的计划都能解析 |
