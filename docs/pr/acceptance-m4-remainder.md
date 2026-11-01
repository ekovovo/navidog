# M4 剩余工作验收

日期：2026-10-31。分支 `feat/m4-remainder`，从 `main` 的 `43dd4c6` 拉出。规格提交 `f36f270`，实现按切片提交。

## 范围

- 表设计器补上排除约束、触发器和分区。PostgreSQL 现有列重排、identity 的添加/删除/模式切换、SQLite 生成列、已有表 `WITHOUT ROWID`、以及字段重命名和其他改动一次保存，都走事务内的整表重建。预览写明这是整表重建；转成 `WITHOUT ROWID` 时写明隐式 rowid 会丢掉。
- 视图、实体化视图、函数与过程、事件、数据库与模式、SQLite 附加、表空间，以及清单点名的其他对象共用一个设计器：名称、选项、定义、SQL 预览、保存。失败留下草稿。查询创建工具仍留在 M7。
- 用户、角色、权限管理员、维护、信息窗格的使用/被使用和成员页、服务器监控、在库中查找。
- SQLite 用户认证扩展没有编进发行版。面板说明原因，不执行 SQL。F-SEC-005 保持部分。

## 刻意不做

运行态 Navicat 并排、字段拖放、引擎私有选项表单、加密 SQLite、SQLite 2、把用户认证扩展编进发行版、自动丢掉依赖对象、接受未知表选项、在附加库上重建表、在生成表达式里猜测标识符。

## 验证

2026-10-31 在本机跑完。截图留在 `target/snapshots/`，不进 git。

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --all --check` | 通过 |
| `cargo clippy --workspace --all-targets --offline -- -D warnings` | 通过 |
| `scripts/check-raw-px.sh`、`scripts/check-i18n.sh`、`scripts/check-dialect-branches.sh`、`scripts/check-deps.sh` | 通过 |
| `cargo test --workspace --offline` | 通过。打开未改动的对象设计器时关闭按钮曾被藏起来，`dirty()` 按已加载对象判断之后 `close_tabs` 通过 |
| `docker compose up -d --wait && NAVIDOG_LIVE=1 cargo test -p navidog-db -p navidog-schema -p navidog-query --lib --test live --offline`，随后 `docker compose down` | 通过。含 `live_type_names_round_trip_from_the_server` 和 `live_table_design_create_alter_rename_and_constraints` |
| `cargo test -p navidog --test snapshot --offline` | 通过。浅色和深色的对象设计器、用户面板已逐张查看：新建视图有名称、定义和 SQL 预览；SQLite 用户面板只显示未编入认证扩展的说明 |

规格验收清单里需要实库往返的条目仍未勾选：排除约束、触发器、分区的执行往返；视图替换与预览数据、实体化视图刷新、函数运行、事件、库/模式/附加、扩展/序列/类型往返；用户创建、授权撤销、维护、依赖页、结束进程、查找双击。这些路径有计划测试或界面，没有单独的实库往返。
