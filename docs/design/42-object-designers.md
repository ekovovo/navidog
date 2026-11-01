# M4 规格：对象设计器

覆盖 F-DSG-011 至 F-DSG-020，以及导航里双击打开设计器、表下字段重命名（F-NAV-002 里推迟到 M4 的两项）。查询创建工具留在 M7，按钮不出现。视图的解释、美化、从文件导入不在清单里，不做。

语句计划在 `navidog-schema`，界面在 `navidog-designer` 的一个面板。外壳按对象种类打开它。不比较 `Dialect::id()`。MariaDB 序列只在连接驱动是 MariaDB 且服务器版本至少 10.3 时出现。

## 1. 共用面板

选项卡接入 `navidog_ui::tabs`。工具栏：保存（⌘S，先打开只读 SQL 预览）、执行（只在预览页）。视图和实体化视图另有「预览数据」，跑 `SELECT * FROM 对象`，结果用现有只读网格，最多沿用查询结果的上限。函数和过程另有「运行」：有参数时弹出对话框（名称、值、原始模式复选框），原始模式不给值加引号；结果沿用查询结果页，最多 20 个结果集。

页：定义、选项、SQL 预览。定义是多行 SQL 编辑器，补全用查询编辑器那一套关键字和已加载的标识符。选项用 40 第 8 节的 FormRow。名称为空不能预览。执行失败保留草稿和错误。未改动关闭不询问。外部对象被删或定义变了时拒绝覆盖，要求重新加载。

双击视图、函数、事件、实体化视图和其他对象打开设计器，不再打开只读 DDL 选项卡。DDL 仍在信息窗格。表的双击仍打开数据查看器。对象工具栏的「新建」「设计」对当前分类可用。表下字段右键「重命名」生成 `ALTER TABLE … RENAME COLUMN`，双击字段打开该表的设计器并选中该字段。

## 2. 视图与实体化视图

定义是一条 `SELECT`。选项：

- MySQL / MariaDB 视图：算法（`UNDEFINED` / `MERGE` / `TEMPTABLE`，缺省空表示不写）、安全性（`DEFINER` / `INVOKER`）、检查选项（空 / `CASCADED` / `LOCAL`）。
- PostgreSQL 视图：无额外选项。实体化视图另有「填充」（缺省勾选，对应 `WITH DATA` / `WITH NO DATA`）。
- SQLite 视图：无额外选项。

保存新对象用 `CREATE VIEW` / `CREATE MATERIALIZED VIEW`。已有对象：MySQL 用 `CREATE OR REPLACE VIEW`；PostgreSQL 视图用 `CREATE OR REPLACE VIEW`；改了实体化视图的查询时 `DROP` 再 `CREATE`（同一事务）；SQLite `DROP VIEW` 再 `CREATE VIEW`。右键「刷新实体化视图」发 `REFRESH MATERIALIZED VIEW name` 或 `… WITH NO DATA`。

## 3. 函数与过程

向导和设计器是同一张选项表，不另做可跳过的向导。类型：过程或函数。参数网格：模式（`IN` / `OUT` / `INOUT`，SQLite 没有过程）、名称、类型；PostgreSQL 另有类型模式和默认值。函数有返回类型。定义是例程体。

MySQL / MariaDB 生成 `CREATE PROCEDURE` 或 `CREATE FUNCTION`，已有同名例程先 `DROP`。PostgreSQL 用 `CREATE OR REPLACE FUNCTION` / `CREATE OR REPLACE PROCEDURE`。SQLite 没有存储例程，函数分类不提供新建。运行：函数 `SELECT name(args)`；过程 `CALL name(args)`。

## 4. 事件 [My]

定义是过程体。选项：状态（`ENABLE` / `DISABLE` / `DISABLE ON SLAVE`）、计划种类（一次 / 每隔）。一次填写时间戳；每隔填写间隔（如 `1 HOUR`）和可选的开始、结束。生成 `CREATE EVENT name ON SCHEDULE … DO body`。已有事件用 `ALTER EVENT`。

## 5. 数据库、模式、附加

- MySQL / MariaDB：新建和编辑数据库。名称、字符集、排序规则。`CREATE DATABASE` / `ALTER DATABASE`。不重命名。
- PostgreSQL：数据库的名称、拥有者、编码、模板（缺省 `template1`）。模式的名称、拥有者。`CREATE` / `ALTER`。重命名用 `ALTER … RENAME TO`。
- SQLite：主库不新建。右键附加库：名称和文件路径，`ATTACH DATABASE file AS name`。分离：`DETACH DATABASE name`，主库没有这一项。不加密、不解密。右键「查看主表」打开 `sqlite_master` 的只读网格。

## 6. 表空间与其他对象

都是名称加少量选项加定义，SQL 预览后执行。已有对象能 `CREATE OR REPLACE` 的用它，否则同一事务里 `DROP` 再 `CREATE`。`DROP` 失败则保留草稿。

| 对象 | 库 | 选项 | 语句 |
| --- | --- | --- | --- |
| 表空间 | MySQL、PostgreSQL | 引擎（MySQL）、数据文件或位置 | `CREATE TABLESPACE` |
| 序列 | PostgreSQL、MariaDB 10.3+ | 起始、递增值 | `CREATE SEQUENCE` |
| 类型 | PostgreSQL | 种类：基本、复合、枚举、范围 | `CREATE TYPE` |
| 域 | PostgreSQL | 基类型、是否空、默认值 | `CREATE DOMAIN` |
| 扩展 | PostgreSQL | 可用列表与已安装列表，移动即安装或删除 | `CREATE EXTENSION` / `DROP EXTENSION` |
| 外部服务器 | PostgreSQL | 包装器、选项 | `CREATE SERVER` |
| 聚合 | PostgreSQL | 输入类型、状态函数 | `CREATE AGGREGATE` |
| 转换 | PostgreSQL | 源类型、目标类型、函数 | `CREATE CAST` |
| 运算符 | PostgreSQL | 左、右、结果类型、函数 | `CREATE OPERATOR` |
| 排序规则 | PostgreSQL | 提供者（`libc` / `icu`）、区域 | `CREATE COLLATION` |
| 索引、触发器 | SQLite，独立于表 | 表名、定义 | `CREATE INDEX` / `CREATE TRIGGER` |

扩展管理是左右两个列表，不是定义编辑器。运算符类别和语言不在清单里，不做。解析不了的已有定义整段放进定义框，保存时原样执行用户改过的语句；没改则不执行。

## 7. 验收

- [ ] 视图在三种库可新建、替换、预览数据；MySQL 高级选项出现在 SQL 里；创建工具不出现。
- [ ] PostgreSQL 实体化视图可新建、刷新、预览。
- [ ] MySQL 与 PostgreSQL 的函数和过程可新建并运行；带参数时先询问。
- [ ] MySQL 事件可按一次和每隔创建。
- [ ] 数据库和 PostgreSQL 模式可新建和编辑；SQLite 可附加、分离、查看主表。
- [ ] 第 6 节的对象至少各有一条计划测试；扩展、序列、类型在对应实库往返。
- [x] 双击非表对象打开设计器；字段可重命名，双击字段打开表设计器。
