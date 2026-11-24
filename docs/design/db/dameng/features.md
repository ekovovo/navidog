# 达梦 DM8 专属功能

达梦高度兼容 Oracle，清单参照 [oracle/features.md](../oracle/features.md) 并按 DM8 实测能力裁剪。通用功能以 [功能清单](../../01-feature-inventory.md) 为准，方言差异与首版边界见 [140-dameng.md](../../140-dameng.md)。

帮助页一栏：本机 `reference/` 尚未导出，页名按 Oracle 同类页命名惯例暂定，标「待核」，导出后核对修正。首版不实现的功能在摘要中标「后续」。

| 编号 | 功能 | 摘要与验收要点 | 帮助页 |
| --- | --- | --- | --- |
| F-DM-001 | 连接常规设置 | 主机、端口（默认 5236）、用户名、密码。单数据库（`V$DATABASE.NAME`），无服务名/SID/初始数据库输入；SYSDBA 是用户名而非连接角色。SSL 首版不支持（dm-pure 无 TLS）。 | con_gen_settings_dameng（待核） |
| F-DM-002 | 模式 | 用户≈模式；以模式为中心的对象导航。系统模式（SYS、SYSDBA、SYSAUDITOR、SYSSSO、CTISYS、INFORMATION_SCHEMA）随系统对象开关过滤。模式切换用 `SET SCHEMA`。 | dameng_schema（待核） |
| F-DM-003 | 表设计器差异 | IDENTITY 列、STORAGE 子句、表空间（MAIN）、`COMMENT ON` 注释。**DDL 隐式提交**：迁移不可回滚，首版限制见 140-dameng 第 6 节。 | dameng_table（待核） |
| F-DM-004 | 视图 | `ALL_VIEWS.TEXT` 读定义；DDL 由 `CREATE VIEW` 拼接。实体化视图后续。 | dameng_view（待核） |
| F-DM-005 | 过程与函数 | `ALL_PROCEDURES`/`ALL_SOURCE`；DMSQL 兼容 PL/SQL。包（PACKAGE）后续。 | dameng_function（待核） |
| F-DM-006 | 其他对象 | 序列（`ALL_SEQUENCES`）、触发器（`ALL_TRIGGERS`）、索引（`ALL_INDEXES`）、同义词（`ALL_SYNONYMS`，后续）。类型、数据库链接、目录后续。 | dameng_others（待核） |
| F-DM-007 | 维护 | 分析表/索引（`ANALYZE`）、重建索引（`ALTER INDEX … REBUILD`）、统计信息。后续（首版维护菜单为空，140 §6.5）。 | dameng_maintain（待核） |
| F-DM-008 | 表空间 | DM 表空间（MAIN、SYSTEM、ROLL、TEMP 等）只读列表与属性；创建/修改后续。 | dameng_tablespace（待核） |
| F-DM-009 | 安全性 | 用户、角色；系统权限与对象权限；维护用户（锁定、解锁、过期密码）。首版只读导航，设计器后续。 | security_dameng_user, security_dameng_role（待核） |
| F-DM-010 | 命令列界面差异 | 首版复用通用命令列；disql 风格命令后续。 | console |
