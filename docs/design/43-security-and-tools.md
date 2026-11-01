# M4 规格：安全、维护与工具

覆盖 F-SEC-001 至 F-SEC-005、F-OBJ-009、F-INFO-003、F-INFO-008、F-TOOL-001、F-TOOL-002。语句在方言方法上，界面按能力显示，不比较 `Dialect::id()`。表单沿用 40 第 8 节的 FormRow。帮助页没有给出的秒数标为暂定。

## 1. 用户与角色

MySQL / MariaDB 用户（F-SEC-001）页：常规（用户名、主机、插件、密码、确认密码）、高级（每小时最大查询、更新、连接，最大用户连接，0 表示不限制；SSL 类型 `NONE` / `ANY` / `X509` / `SPECIFIED`）、成员属于（角色网格：授予、默认）、服务器权限（授予复选）、对象权限（展开到表，勾选权限）、SQL 预览。保存生成 `CREATE USER` 或 `ALTER USER`，加上 `GRANT` / `REVOKE`。密码只在本次填写后写入语句，不回读。角色（F-SEC-002）同一面板，没有主机、插件和密码：名称、成员、服务器权限、对象权限、SQL 预览。

PostgreSQL 用户、角色、组（F-SEC-003）共用一个面板。选项：名称、可登录、密码、超级用户、创建数据库、创建角色、继承。成员和成员属于是两个名单。权限网格按对象列出后生成 `GRANT` / `REVOKE`。组就是 `NOLOGIN` 的角色。SQL 预览后执行，失败保留草稿。

SQLite 用户（F-SEC-005）：用户名、密码、确认密码、管理员。打开时探测用户认证扩展；没有扩展就不执行语句，面板说明发行版的 SQLite 不带该扩展。不把扩展编进发行版。

## 2. 权限管理员

F-SEC-004。MySQL / MariaDB 和 PostgreSQL 可用，SQLite 不显示。从对象出发：左边是对象树，选中后右边是用户或角色，每个权限一个复选框。变更生成 `GRANT` / `REVOKE`，先预览再执行。

## 3. 维护

F-OBJ-009。对象或库的右键「维护」。结果在对话框里列出服务器返回的行。只提供清单里的命令：

- MySQL / MariaDB 表：`ANALYZE TABLE`、`CHECK TABLE`、`OPTIMIZE TABLE`、`REPAIR TABLE`。
- PostgreSQL：`VACUUM`、`ANALYZE`、`REINDEX`。目标是选中的表，没有选中表时是当前库。
- SQLite：`VACUUM`、`ANALYZE`、`REINDEX`、`PRAGMA integrity_check`。`VACUUM` 只对主库。

## 4. 信息窗格

使用与被使用（F-INFO-003）：两列名单。PostgreSQL 读 `pg_depend`。MySQL 从表、视图、例程和触发器定义里匹配引用到的名字，匹配不到就空着，不猜测。SQLite 读 `sqlite_master` 的 SQL。点一项在导航里选中该对象。

成员、成员属于、权限（F-INFO-008）：只在用户或角色上出现。成员属于、成员、权限各一页，只读，和设计器读的是同一份目录。

## 5. 服务器监控

F-TOOL-001。「工具 > 服务器监控」。MySQL / MariaDB 和 PostgreSQL 可用，SQLite 没有这一项。三页：进程、变量、状态。

- 进程：MySQL `SHOW FULL PROCESSLIST`，PostgreSQL `pg_stat_activity`。选中后可结束（`KILL` / `pg_cancel_backend`）。
- 变量：MySQL `SHOW VARIABLES`，可编辑并 `SET GLOBAL`。PostgreSQL `SHOW ALL`，只读。
- 状态：MySQL `SHOW GLOBAL STATUS`，PostgreSQL 读 `pg_stat_database` 里当前库那一行。

自动刷新缺省关闭。菜单可打开，间隔暂定 5 秒（帮助页只说输入秒数，没有缺省）。窗口关掉即停。

## 6. 在数据库或模式中查找

F-TOOL-002。「工具 > 在数据库或模式中查找」，快捷键 ⇧⌘F。连接、数据库、模式（仅有模式的库）、查找字符串、范围（数据 / 结构）、模式（包含、完整单字、前缀、正则表达式）、不区分大小写。结构范围可选表、视图、函数、索引、触发器、事件、实体化视图中当前库有的种类。数据范围只查表和视图的文本列，每个对象最多 100 行命中。结果双击：结构打开设计器，数据打开表查看器。SQLite 可以查。正则表达式非法时在点查找时说明，不执行。

## 7. 验收

- [ ] MySQL 用户和角色、PostgreSQL 角色可预览并在实库创建；密码不回读。
- [ ] 权限管理员对一张表授予并撤销一项权限。
- [x] SQLite 用户面板在没有扩展时给出说明且不执行 SQL。
- [ ] 三种库的维护命令各有一条实库或本地测试。
- [ ] 使用与被使用、成员页能列出依赖和角色关系。
- [ ] 监控能列出进程并结束一条测试连接；自动刷新可关。
- [ ] 查找能命中一条表数据和一个对象名，双击打开对应面板。
