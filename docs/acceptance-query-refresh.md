# 查询执行后的对象与表格刷新

分支：`codex/query-auto-refresh`。基线为 `upstream/main` 的 `3a3ab64`；行为规格先在 `97bd23d` 独立提交，关闭窗口的生命周期约束补充于 `572f452`，见 20、21、31 三份模块规格。推送前同步了 upstream 的历史重写，与原验证基线 `9f3a926` 相比只删除两张旧截图和相关说明，生产代码未改变。

## 行为与边界

- 查询编辑器每次实际执行普通 SQL 后，对执行时的连接发送一次通知；多条语句部分成功后失败或取消仍会通知。解释工具、未完成的命令列输入和未开始执行的查询不发送通知。
- 已提交的后台 SQL 不因关闭查询/命令列窗口而停止；完成通知由独立于面板和窗口的任务等待实际数据库任务结束后发送，不额外保持面板存活。
- 命令列界面走相同通知路径。不通过 SQL 首关键字推断影响范围，避免漏掉跨库名称、`WITH`、函数、触发器或过程的副作用。
- 导航目录重载同连接已有的数据库、模式、对象、字段、索引/外键/触发器和 DDL 缓存；不主动遍历未加载节点。已打开的只读 DDL 面板随缓存更新。不同连接隔离，过期请求不能覆盖新缓存。
- 干净的表查看器在自己原有会话上重读字段、外键、当前页和记录数，保留筛选、排序、分页及显示设置。加载中收到的通知合并为一次后续刷新。
- 未提交修改、空白新增行、活动单元格/日期编辑、尚未应用的表单/字段输入和查看器事务阻止自动刷新，保留现场并在状态栏提示处理后刷新。刷新进行中输入的表单/字段草稿同样保留，应用和切记录被禁用；后续显式刷新会重读字段与外键。
- 不自动提交或回滚，不绕过数据库隔离：查询事务提交前，其他会话可能仍看到旧内容；执行 `COMMIT` 后再次通知。既有查询结果（包括固定结果）保留为当次执行的快照。

## 可运行回归

| 检查 | 证明内容 |
| --- | --- |
| `query_toolbar_refreshes_open_tables_and_view_definitions` | 使用真实 SQLite，通过查询工具栏运行 UPDATE 和重建视图；通过表格复制内容及真实只读编辑器文档值验证原有窗口更新，未派发手动刷新 |
| `database_changes_reload_cached_metadata_and_open_ddl_without_touching_other_connections` | 表、视图、字段、索引及打开的 DDL 更新；其他连接和未加载节点保持隔离 |
| `a_replaced_metadata_load_cannot_restore_older_values` | 人为延迟旧请求，验证完成后不能恢复旧缓存 |
| `query_runs_publish_one_refresh_for_the_original_connection` | 通知合并、部分失败、事务提交/回滚、解释不通知、执行中切连接及取消 |
| `closing_query_and_console_windows_preserves_in_flight_refreshes` | 用真实 SQLite 写锁阻塞 UPDATE，关闭窗口并确认面板已释放后再解除锁，验证写入成功且各只通知一次 |
| `history_log_follows_the_log_and_the_console_runs_statements` | 命令列的未完成输入、普通执行、取消和部分成功后的通知 |
| `visibility_uses_accepted_updates_inserts_and_deletes` | 表格数据和 ALTER/FK 更新、连接隔离、加载期间通知合并、分页筛选排序保留、各类编辑/事务保护，以及刷新期间继续输入的竞态 |

## 依赖审计

验证时在线审计发现两个新公布的问题，使用补丁版本解决：`ctutils` 0.4.3 → 0.4.4（RUSTSEC-2026-0385），`tokio` 1.53.2 → 1.53.3（RUSTSEC-2026-0386）。锁文件仅修改这两个包的版本及校验和，未新增忽略项。

## 全量验证记录

2026-10-24，Rust 1.99.0，本机 Apple Silicon：

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --all --check` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过；依赖的 future-incompatibility 提示仍在，不是本次新增 |
| `scripts/check-raw-px.sh` | 通过 |
| `cargo audit` | 通过；9 项传递依赖的允许警告仍在，无新增忽略项 |
| `cargo deny --all-features check` | advisories、bans、licenses、sources 均通过 |
| `NAVIDOG_LIVE=1 cargo test --workspace --locked` | 37 个测试组共 226 项通过；包含 DB 实库 15 项、MySQL/MariaDB/PostgreSQL 查询操作和计划解析 |
| `cargo test -p navidog --test snapshot --locked` | 39 张 PNG 已逐张检查，含新增的自动刷新和保留编辑提示各一对浅深色图 |
| release 网格 10 万行 × 20 列 | 逐行/跳跃平均 3.25 / 4.57ms，p95 4.32 / 5.99ms，超过 8.3ms 的帧均为 0/300 |
| release 导航树 5000 项 | 逐行/跳跃平均 0.56 / 0.82ms，p95 1.21 / 1.37ms，超过 8.3ms 的帧均为 0/300 |
| release SQL 编辑器 5 万行 | 滚动、带装饰滚动及连续输入平均约 1.57–1.59ms，超过 8.3ms 的帧均为 0/300 |

实库测试前执行 `docker compose up -d --wait`，结束后执行 `docker compose down`。记录保存在本地 `target/query-refresh-checks/`。截图只写入本地 `target/snapshots/`，不提交 PNG：

- `viewer-auto-refresh-light.png`、`viewer-auto-refresh-dark.png`：真实 SQLite 的外部 ALTER/UPDATE/INSERT 后，已开表显示新增字段和记录。
- `viewer-auto-refresh-pending-light.png`、`viewer-auto-refresh-pending-dark.png`：外部 UPDATE 不覆盖本地草稿，底栏提示保持单行，分页和记录数可见。

本次不把 headless 操作测试或截图当作真实 macOS 窗口与 Navicat 并排人工验收；原有里程碑里的人工目测遗留保持原状态。
