# 140 达梦 DM8 接入

M14 的第一个数据库。新建不依赖 GPUI 的 `navidog-dameng` crate，实现 `Driver`/`Session`/`Relational`/`Dialect`，在 `navidog-session::init` 注册。界面零分支：连接表单只读 `DriverInfo` 标志，grid/query/designer/schema 只问能力方法。专属功能清单见 [db/dameng/features.md](db/dameng/features.md)，技术验证证据见 [pr/acceptance-dameng.md](../pr/acceptance-dameng.md)。

驱动：`dm-pure`（MIT，纯 Rust 同步 API）。阶段 0 判据 1–4 全部通过，ODBC 兜底不启用。dm-pure 1.2.4 有四处协议缺陷，已定位并验证补丁（验收记录「dm-pure 缺陷与补丁」节）。上游仓库 `zhangtao103239/dm8_pure_rs` 已从 GitHub 消失（404，全站含 fork 搜索无镜像），暂无法 fork/提 PR。**依赖策略（已确认）**：补丁托管在自建公开镜像 `github.com/ekovovo/dm8_pure_rs`（`main` = crates.io 1.2.4 原版镜像，`fix/protocol-lob-explain-error-status` 分支仅含四补丁），`navidog-dameng` 经 `[patch.crates-io]` git 依赖引用修复分支；上游恢复后向其提 PR，上游发布修复版后切回 crates.io 精确版本。

## 1. 范围与首版边界

首版实现：

- 连接：常规（主机/端口/用户/密码）、SSH 隧道（`russh`，复用 `navidog_db::connect` 编排）。SSL 不支持（第 7 节偏差 D3）。
- 导航树到对象级：模式 → 表、视图、函数、过程、序列、触发器、索引（表下）；系统模式随系统对象开关过滤。用户/角色/表空间只读列表（F-DM-008/009 的设计器后续）。
- 表查看器与网格编辑：分批读取（`execute_lazy` + `fetch_more`，首批 100 行）、单元格编辑、提交流程（`navidog_db::commit`，SAVEPOINT 已验证兼容）。
- 查询编辑器：执行、停止（第 3.4 节）、解释（第 5.6 节）、历史日志（复用 `navidog-sql-log`，无方言分支）。
- 表设计器：单列第 6 节，首版限制迁移能力。

首版不做（后续里程碑或 M14 后续任务）：实体化视图、包、同义词/类型/数据库链接设计器、分区设计器、函数索引、虚拟列、用户/角色设计器、表空间创建修改、disql 风格命令列、数据泵/回收站（DM 未验证）。

## 2. 连接参数与 DriverInfo

```text
id: "dameng"
name: "Dameng"（zh-CN: 达梦）
default_port: 5236
file_based: false
supports_network: true
supports_ssl: false        // dm-pure 无 TLS（偏差 D3）
supports_ssh: true
multi_host: false
protocol_timeouts: false
connection_params: false
per_database_sessions: false
context_selection: Schema  // 单数据库固定，SET SCHEMA 切换模式
```

表单复用现有标志逻辑，**无需改动**：

- 常规页：`supports_network=true` 且 `multi_host=false` → 主机、端口、用户、密码；初始数据库输入只在 `multi_host=true` 时显示，DM 不显示（单数据库，连接后取 `V$DATABASE.NAME`）。
- 高级页：`protocol_timeouts=false`、`connection_params=false` → 只有通用项（设置位置、keepalive、自动连接等），无协议超时组、无连接参数表。
- SSL 页：`supports_ssl=false` → 不显示。SSH 页照常。
- 连接超时：`ConnectParams.timeout()` 映射到 `ClientConfig.timeout`（连接与读取共用，dm-pure 无独立读取超时）。`auto_reconnect` 固定 `false`（重连由 `navidog-db` 的恢复宏编排，驱动不自作主张）。

图标：决策 3 要求已授权第三方 SVG。Tabler/Devicon 均无达梦，**已确认自绘抽象图形**：原创 SVG，尺寸/位置与现有数据库图标一致，不照抄达梦商标图形，不提取 Navicat 图标。

## 3. 会话与执行

### 3.1 线程模型

照 `navidog-sqlite`：`Arc<Mutex<DmClient>>` + `navidog_db::runtime::spawn_blocking`，watch channel 做 workers 退休栅栏。dm-pure 是同步 API，所有协议调用在阻塞线程上；界面经 `Sessions::run_rel` / `sessions::run_on` 在 `cx.spawn` 中等待。

### 3.2 Session 方法

| 方法 | 实现 |
| --- | --- |
| `server_version` | `SELECT BANNER FROM V$VERSION WHERE ROWNUM = 1` + `SELECT ID_CODE()`，空格拼接（如 `DM Database Server 64 V8 --03134284294-20241009-244896-20119 Pack3`）。连接时取一次缓存。 |
| `ping` | `SELECT 1 FROM DUAL`，走 `recovering_ping!`。 |
| `session_id` | `SELECT SESSID()`（BigInt → 字符串）。`SF_GET_SESSION_ID()` 不存在，不用。 |
| `cancel_handle` | 见 3.4。 |
| `close` | `DmClient::close()`（协议级）。 |
| `relational` | 返回自身。 |

连接成功后立即探测（一次往返各自独立查询，失败不阻塞连接，回退默认值并记日志）：

- `SELECT CASE_SENSITIVE()` → 1/0（`V$DM_INI` 无此行，必须用函数）。
- `SELECT PARA_VALUE FROM V$DM_INI WHERE PARA_NAME = 'COMPATIBLE_MODE'` → 0/2/4。

探测结果构造会话级 `DamengDialect`（第 5 节）；`DriverInfo.dialect` 是默认实例（CS=1、mode 0），供连接前界面使用。

### 3.3 执行语义与恢复

`execute` / `execute_read` / `execute_statement` 按 `navidog-db` driver.rs 注释区分（读结果集 / 受影响行数 / 语句执行）。dm-pure `ExecuteResult::Query` → `ResultSet`（列元数据来自 `Column`，行值按第 5.5 节映射）；`Affected(n)` → 受影响行数；`Call` → 按存储过程结果处理（首版 `CALL` 走 `execute_read`，多结果集后续）。

错误映射 `DbError { code, message, disconnected, outcome_unknown }`：

- `code`：dm-pure 错误码取负号还原服务器码（如 -2501、-2106、-6852）。
- `disconnected=true`：IO 错误、`-6852`（连接关闭/网络异常）。取消杀会话后 victim 收 -6852，属预期断开。
- `outcome_unknown=true`：写入发出后连接断开且无法确认是否提交（恢复宏据此守卫，不重放普通 SQL）。
- 消息：补丁 3 后为干净服务器文本，直接透传（crate 内不做产品名分支）。

恢复走 `recovering_execute!`/`recovering_ping!`：attempt → lost（disconnected）→ reconnect（新建 DmClient，重新探测）→ remember → recovery。元数据会话可重连重试；用户写入不重放。

### 3.4 取消（偏差 D2）

dm-pure 无协议级取消。实现：`cancel_handle` 持有连接参数副本，取消时**另开一个短期连接**执行 `CALL SP_CLOSE_SESSION(<sessid>)`，杀掉目标会话。后果：

- 目标会话整体关闭，正在执行的语句中断（victim 收 -6852）。
- 原会话不可复用，恢复宏按 disconnected 重连；重连后事务状态丢失，按 05-architecture-foundations 的事务丢失守卫提示。
- 取消用的短期连接用完即关，不进会话池。

不采用「只中断语句」的假实现；偏差与理由写第 7 节 D2。

## 4. 目录 SQL

八个目录方法经 `navidog_db::relational_catalog!(metadata)` 转发到 `metadata.rs`；测试替身用 `unsupported_catalog!`。所有目录查询用绑定参数（`Param::Str`），OWNER 过滤模式名。

| 方法 | SQL 来源 |
| --- | --- |
| `list_databases` | `SELECT NAME FROM V$DATABASE` → 单元素（`DAMENG`）。 |
| `list_schemas` | `SELECT USERNAME FROM ALL_USERS ORDER BY USERNAME`。用户≈模式。 |
| `list_objects(Table)` | `ALL_TABLES`（NUM_ROWS、OWNER）+ `ALL_TAB_COMMENTS` 注释。 |
| `list_objects(View)` | `ALL_VIEWS WHERE OWNER = :s`。 |
| `list_objects(Function/Procedure)` | `ALL_PROCEDURES WHERE OWNER = :s AND OBJECT_TYPE = 'FUNCTION'/'PROCEDURE'`（`ObjectKind::Function` 承载，`ObjectInfo` 区分 routine 种类，照 PG 惯例）。 |
| `list_objects(Sequence)` | `ALL_SEQUENCES WHERE SEQUENCE_OWNER = :s`。 |
| `list_objects(Trigger)` | `ALL_TRIGGERS WHERE OWNER = :s`。注意 DM 的触发器种类列是 `TRIGGERING_TYPE`（不是 Oracle 的 `TRIGGER_TYPE`）。 |
| `list_objects(Index)` | 表下子对象，见 `table_children`；模式级不列（`standalone_indexes=true` 但导航照 Oracle 惯例挂表下，首版 `object_kinds` 不含模式级 Index）。 |
| `object_ddl(Table)` | `CALL SP_TABLEDEF(:s, :t)` → 单行 Str（含 STORAGE 子句、`\r\n`，原样展示）。`SP_TABLEDEF` 不能作为 SELECT 项（-4015），只能 CALL；驱动把 `ExecuteResult::Call` 的 OUT 值映射成一行。`DBMS_METADATA.GET_DDL` 不可用（TABLE 报 -20008，其余返回空串）。 |
| `object_ddl(View)` | `SELECT TEXT FROM ALL_VIEWS WHERE OWNER = :s AND VIEW_NAME = :v`，拼 `CREATE VIEW "s"."v" AS\n<text>`。 |
| `object_ddl(Function/Procedure)` | `ALL_SOURCE WHERE NAME = :n ORDER BY LINE`，`LISTAGG` 拼接。实测 DM 把函数源码的 `TYPE` 记为 `'PROC'`（不是 Oracle 的 `'FUNCTION'`），过滤条件用 `TYPE IN ('FUNCTION', 'PROC')`。 |
| `object_ddl(Sequence)` | 由 `ALL_SEQUENCES`（MIN_VALUE、MAX_VALUE、INCREMENT_BY、CYCLE_FLAG、CACHE_SIZE、LAST_NUMBER）合成 `CREATE SEQUENCE`。 |
| `object_ddl(Index)` | 由 `ALL_INDEXES` + `ALL_IND_COLUMNS` 合成 `CREATE [UNIQUE] INDEX … ON …(…)`。 |
| `object_ddl(Trigger)` | `ALL_TRIGGERS` 的 DESCRIPTION/TRIGGER_BODY 合成 `CREATE OR REPLACE TRIGGER …`（实测 DESCRIPTION 含完整触发头）。 |
| `table_columns` | `ALL_TAB_COLUMNS WHERE OWNER AND TABLE_NAME`（COLUMN_NAME、DATA_TYPE、DATA_LENGTH、DATA_PRECISION、DATA_SCALE、NULLABLE、DATA_DEFAULT、COLUMN_ID）+ `ALL_COL_COMMENTS` 注释 + 主键（`ALL_CONSTRAINTS`/`ALL_CONS_COLUMNS` CONSTRAINT_TYPE='P' 的 EXISTS 子查询）。**`ALL_TAB_COLUMNS` 没有 `IDENTITY_COLUMN` 列**（阶段 2 实测，规格初稿的候选不存在），identity 见 5.4 的 `column_metadata_sql`。 |
| `table_children` | 索引：`ALL_INDEXES`（排除 `INDEX_TYPE='CLUSTER'` 的 CLUSTERBTR 存储索引）+ `ALL_IND_COLUMNS`（表级）；触发器：`ALL_TRIGGERS WHERE TABLE_OWNER AND TABLE_NAME`；外键/检查见下。 |
| `foreign_keys` | `ALL_CONSTRAINTS`（CONSTRAINT_TYPE='R'）+ `ALL_CONS_COLUMNS` + 以 `R_CONSTRAINT_NAME` 回查被引用表/列。 |
| `outline` | 补全大纲：`ALL_TABLES` + `ALL_VIEWS` + `ALL_TAB_COLUMNS`（当前模式，system=true 时含系统模式），照 PG 的 outline 结构。 |

目录查询绑定方式：`Relational::execute_read` 没有参数通道，照 PG 的惯例用 `string_literal` 的引号字面量过滤 OWNER/表名（规格初稿写的"绑定参数"按此调整）。

检查约束：`ALL_CONSTRAINTS CONSTRAINT_TYPE='C'` 的 `SEARCH_CONDITION`（设计器与 DDL 用）。

系统模式（`Dialect::system_schemas`）：`SYS`、`SYSDBA`、`SYSAUDITOR`、`SYSSSO`、`CTISYS`、`INFORMATION_SCHEMA`。系统对象开关关闭时导航过滤这些模式下的对象。

## 5. 方言差异

`DamengDialect` 携带探测结果构造：`DamengDialect { case_sensitive: bool, compatible_mode: u8 }`。`Default` = CS=1、mode 0（`DriverInfo.dialect` 用）；会话级 `Relational::dialect()` 返回探测后的实例。**禁止比较 `Dialect::id()`**，差异全部走下面的能力字段与返回 SQL 的方法。

### 5.1 词法（Lex）

| 标志 | 值 | 说明 |
| --- | --- | --- |
| `double_quote_idents` | true | `"ident"`，内部 `""` 转义。 |
| `backtick_idents` | `compatible_mode == 4` | 仅 MySQL 兼容模式支持反引号（mode 0/2 报 -2007，实测）。 |
| `hash_comments` | false | 无 `#` 注释。 |
| `dollar_quotes` | false | 无 PG 式 `$tag$`。 |
| `bracket_idents` | false | 无 `[ident]`。 |
| `backslash_escapes` | false | 字符串内反斜杠不转义（Oracle 惯例）；mode 4 下未实测，阶段 2 核对，如有差异按 mode 取值。 |
| `national_literals` | false | 首版不识别 `N''`（未实测）。 |
| `escape_strings` | false | 无 `E''`。 |
| `optimizer_hints` | true | Oracle 兼容 `/*+ … */`。 |
| `version_comments` | false | 无 MySQL `/*! … */`。 |
| `delim_commands` | false | 无 `DELIMITER`。 |
| `plus_terminators` | false | 首版不识别 `/` 终结符；PL/SQL 块切分依赖扫描器的 `BEGIN…END` 处理（disql 的 `/` 习惯列后续）。 |

### 5.2 标识符引用与大小写（新增能力字段）

`quote_ident`：双引号，内部双引号翻倍。

`DialectFeatures` 新增两个字段（基线 false，语义在此定义）：

1. **`folds_unquoted_upper: bool`**：未引用标识符折叠为**大写**（DM `CASE_SENSITIVE=1`）。与现有 `folds_unquoted`（折小写，PG）互斥，两者不同时为 true。消费者 `typed_name`：该字段为 true 时，含小写字母的标识符必须加引号（服务器把未引用名折大写，引用创建的小写名只有引用才能命中）；含大写而 `folds_unquoted=true` 时加引号（现有逻辑）；两者皆 false（CS=0）时按原样存储，仅关键字/特殊字符需要引号。
2. **`empty_string_is_null: bool`**：服务器把 `''` 当 NULL（DM `COMPATIBLE_MODE=2`，实测 mode 0/4 非 NULL）。消费者：网格单元格编辑器空输入按 NULL 写入；筛选生成把 `= ''` 改写为 `IS NULL`、`<> ''` 改写为 `IS NOT NULL`。首版接这两个消费者，其余场景后续。

取值：CS=1 → `folds_unquoted_upper=true`；CS=0 → 两者 false。mode 2 → `empty_string_is_null=true`；mode 0/4 → false。

### 5.3 DialectFeatures 取值

基线（`Default`）之外的显式取值，未列出的保持基线：

| 字段 | 值 | 理由 |
| --- | --- | --- |
| `has_schemas` | true | 用户≈模式。 |
| `folds_unquoted_upper` | CS=1 | 新字段，5.2。 |
| `empty_string_is_null` | mode 2 | 新字段，5.2。 |
| `transactional_ddl` | **false** | DDL 隐式提交（实测，偏差 D1）。 |
| `drop_index_by_name` | true | `DROP INDEX idx` 独立语句（实测）。 |
| `drop_trigger_by_name` | true | Oracle 惯例 `DROP TRIGGER trg`（阶段 2 实测核对）。 |
| `standalone_indexes` | true | 独立 `CREATE INDEX`（实测，基线即 true）。 |
| `separate_comments` | true | `COMMENT ON TABLE/COLUMN`（实测）。 |
| `supports_column_comments` | true | 基线即 true。 |
| `supports_identity` | true | `IDENTITY(seed, increment)`（实测）；子句语法与 PG 的 `GENERATED … AS IDENTITY` 不同，由方言方法生成（5.4）。 |
| `identity_editable` | **false** | 新字段（基线 true）。设计器「自增」属性行是否可编辑。DM 首版 IDENTITY 参数只读（6.3）；创建 IDENTITY 列经定义文本框。 |
| `enriches_from_catalog` | true | DDL 来自 SP_TABLEDEF，列元数据从目录读（照 PG）。 |
| `console_errors_have_code` | true | 错误带服务器码（基线即 true）。 |
| `console_boxed_tables` | true | disql  boxed 表格风格。 |
| `fk_reference_schema` | true | 外键可跨模式（基线即 true）。 |
| `counts_changed_rows` | false | UPDATE 受影响行数按匹配行计（Oracle 惯例，基线即 false）。 |
| `like_text_cast` | false | 数字/日期隐式转文本做 LIKE（Oracle 惯例，基线即 false）。 |
| `supports_virtual_generated` | **false** | 覆盖基线 true；虚拟列未实测，后续。 |
| `supports_index_method` | **false** | 覆盖基线 true；`USING method` 未实测，首版普通索引。 |
| `supports_index_expression` | **false** | 覆盖基线 true；函数索引后续。 |
| `exposes_rowid` | false | DM 有 ROWID 但首版不暴露（后续）。 |
| `supports_partitions` | false | 分区设计器后续。 |

### 5.4 返回 SQL 的方法

现有方法语法与 DM 不符时新增方言方法（语义先在本节登记，实现由编译器强制补齐）：

| 用途 | DM 语法（实测） |
| --- | --- |
| 开始事务 | `START TRANSACTION`（`BEGIN TRANSACTION` 报 -2004，不用；协议级 begin 仅驱动内部用） |
| 提交/回滚 | `COMMIT` / `ROLLBACK` |
| 保存点 | `SAVEPOINT p` / `ROLLBACK TO SAVEPOINT p` / `RELEASE SAVEPOINT p`（`navidog_db::commit` 现有流程兼容） |
| 切换模式 | `SET SCHEMA "s"`（补丁 4 后可用；`ALTER SESSION SET CURRENT_SCHEMA` 等价，统一用前者） |
| 分页 | `LIMIT n OFFSET m`（四种语法均可用，统一 LIMIT；`TOP`/`ROWNUM`/`FETCH FIRST` 不用） |
| 改列类型/可空/默认 | `ALTER TABLE t MODIFY col <type> [NULL|NOT NULL]`；默认值 `ALTER TABLE t ALTER COLUMN col SET DEFAULT <v>` / `DROP DEFAULT` |
| 加列 | `ALTER TABLE t ADD (col <def>)`（单列也带括号，Oracle 惯例） |
| 删列 | `ALTER TABLE t DROP COLUMN col` |
| 改列名 | `ALTER TABLE t RENAME COLUMN a TO b` |
| 改表名 | `ALTER TABLE t RENAME TO new`（独立 `RENAME` 报 -2007，不用） |
| 加/删约束 | `ALTER TABLE t ADD CONSTRAINT c PRIMARY KEY/UNIQUE/CHECK …`；`ALTER TABLE t DROP CONSTRAINT c` |
| 注释 | `COMMENT ON TABLE t IS '…'`；`COMMENT ON COLUMN t.c IS '…'` |
| IDENTITY 子句 | `IDENTITY(<seed>, <increment>)`（列定义内） |
| 索引 | `CREATE [UNIQUE] INDEX i ON t(c1, c2)`；`DROP INDEX i` |
| 序列 | `CREATE SEQUENCE s START WITH n INCREMENT BY n [MAXVALUE n] [CYCLE|NOCYCLE] [CACHE n]`；取值 `s.NEXTVAL` |
| 解释 | `EXPLAIN <sql>`（5.6） |

现有方法语法与 DM 相符的（`rename_column` 的 `ALTER COLUMN … RENAME TO`、`select_page` 的 `LIMIT … OFFSET`、`drop_constraint_clause` 的 `DROP CONSTRAINT`）保持默认实现，阶段 2 实库回归确认。

**设计器 SQL 钩子（新增方言方法，语义在此登记，实现由编译器强制补齐）：**

表设计器「读 DDL 后从目录富集」路径（`enriches_from_catalog`）目前在 `navidog-schema` 里硬编码 PG 的 `pg_class`/`pg_attribute` 查询与 `GENERATED … AS IDENTITY` 文本。为让 DM 复用同一路径而界面不按产品分支，把这些 SQL/文本生成移进方言方法；默认实现保持 PG 现状，DM 覆盖：

| 方法 | 签名语义 | 默认（PG 现状） | DM |
| --- | --- | --- | --- |
| `table_options_sql` | `(&self, scope: &Scope, name: &str) -> Option<String>`；表选项查询，结果固定 5 列：owner、persistence（`p`/`u`）、存储参数、表空间、inherits | `pg_class` 联 `pg_roles`/`pg_tablespace`（现有 SQL 搬入） | `SELECT OWNER, 'p', '', TABLESPACE_NAME, '' FROM ALL_TABLES WHERE OWNER = … AND TABLE_NAME = …`（内嵌字面量，照 PG；DM 无 unlogged/继承，persistence 恒 `p`） |
| `column_metadata_sql` | `(&self, scope: &Scope, name: &str) -> Option<String>`；列元数据查询，结果固定 9 列：名、类型文本、notnull（`t`/`f`）、默认、identity 目录值、generated 表达式、collation 模式、collation 名、注释 | `pg_attribute` 联 `pg_type`/`pg_collation`/`pg_description`（现有 SQL 搬入） | `ALL_TAB_COLUMNS` + `ALL_COL_COMMENTS`；类型文本由 `DATA_TYPE`/`DATA_LENGTH`/`DATA_PRECISION`/`DATA_SCALE` 合成（如 `VARCHAR(100)`、`DECIMAL(10,2)`、`TIMESTAMP(6)`）；notnull 由 `NULLABLE='N'`→`t`；identity 列携带完整 `IDENTITY(seed, increment)` 文本——**阶段 2 实测**：种子/增量不在任何目录列里，`ALL_TAB_COLUMNS.IDENTITY_COLUMN` 不存在；可用的来源是 `SYS.SYSCOLUMNS.INFO2`（identity 列为 1，列名可关联）加系统函数 `IDENT_SEED`/`IDENT_INCR`（非 identity 表报 -7019，CASE 分支也拦不住它的求值），因此 identity 文本由 `UNION ALL` 的一个分支产出：该分支按 `S.INFO2 = 1` 过滤行，非 identity 表在此分支没有行、函数不执行；无 identity 的值列恒空串；generated/collation 恒空（首版不支持虚拟列与逐列 collation） |
| `identity_definition` | `(&self, catalog: &str) -> String`；把 identity 目录值（上列第 5 格）映射为追加到列定义的子句（非空时带前导空格） | `a`→` GENERATED ALWAYS AS IDENTITY`，`d`→` GENERATED BY DEFAULT AS IDENTITY`，其余空 | 目录值已是 `IDENTITY(seed, increment)` 文本，非空原样返回 ` {value}` |
| `add_column` | `(&self, table, column, definition, position) -> String`；单条 `ALTER TABLE … ADD` | `ALTER TABLE {table} ADD COLUMN {column} {definition}{position}`（现有文本） | `ALTER TABLE {table} ADD ({column} {definition})`（Oracle 式括号；DM 不支持列位置，`position` 恒空，阶段 2 实库回归） |
| `modify_column` | `(&self, table, old_name, new_name, old_definition, new_definition) -> Option<Result<Vec<String>, String>>`；`Some` 表示方言接管「改已有列」整段语句序列，`None` 回退现有 PG 路径 | `None` | `Some`：改名用 `ALTER TABLE … RENAME COLUMN old TO new`；类型/NOT NULL 变化用 `ALTER TABLE … MODIFY (col …)`；默认值用 `ALTER COLUMN col SET DEFAULT …` / `DROP DEFAULT`（均阶段 2 实库回归）；**旧或新定义含 IDENTITY 子句 → `Err`**（首版 IDENTITY 列只读，任何结构变化提示删列重加、数据丢失，6.3） |

两个查询的列布局是 `navidog-schema` 与方言之间的契约，DM 必须产出相同列数与语义，加载器（`load_in_context`）不再按产品分支。两个查询入参取 `scope`+表名（而非已拼好的限定名）：DM 需把 owner（`scope.schema`）与表名分开过滤，PG 在方法内部自拼限定名。`session_setup`（`navidog-db`）目前只认 `USE`/`SET search_path`，新增对 `SET SCHEMA <name>` 的通用识别（`ContextSelection::Schema` 产品），记为会话当前模式，不按产品分支。

### 5.5 类型映射

dm-pure `Value` → 网格文本（`navidog_util::format` 现有工具优先）：

| Value | 显示 |
| --- | --- |
| `Null` | NULL |
| `Str` | 原文（CLOB 同） |
| `Decimal` | 十进制文本 |
| `Bool` | `1`/`0`（BIT） |
| `TinyInt`/`SmallInt`/`Int`/`BigInt` | 整数文本 |
| `Real`/`Double` | 浮点文本 |
| `Bytes`/`Raw` | 十六进制（BLOB/BINARY，网格二进制格式化器） |
| `Date` | `YYYY-MM-DD` |
| `Time` | `HH:MM:SS` |
| `Timestamp` | `YYYY-MM-DD HH:MM:SS[.ffffff]` |
| `Interval` | 服务器原文 |

列元数据：`Column.type_name`（服务器给的名，如 VARCHAR/INT/CLOB）、`prec`/`scale`、`nullable`、`lob`。`ColumnInfo` 的默认值/注释/主键/IDENTITY 从目录补（`enriches_from_catalog=true`）。VARCHAR 超长服务器报错 -6169，不静默截断，网格提交错误原样提示。

### 5.6 解释（新增 ExplainStyle 变体）

`ExplainStyle` 新增 **`TextLines`**：计划是缩进文本行，无结构化节点。语义：`explain_sql` 返回 `EXPLAIN <sql>`；结果按单列文本行展示在查询编辑器的**文本视图**；网格、可视化、统计信息视图对该样式隐藏（`navidog_util::explain` 不解析）。现有三种样式（TableAndJson/JsonNodes/QueryPlanRows）不变。dm-pure 补丁 2 把 cmd=149 帧解析为单列 `PLAN` 行，正好对接。

### 5.7 关键字

`words.rs` 来源：DM8 rev244896 实例的 `V$RESERVED_WORDS`（实测 794 词，含保留/非保留标记），与《DM8 SQL 语言使用手册》（2024-10-22 版，对应镜像 rev244896）保留字表交叉核对。文件头注明来源与导出日期。补全关键字与内置函数照 PG 的 words.rs 结构。

## 6. 表设计器（单列）

### 6.1 DDL 隐式提交的影响

实测：DM 的 DDL 触发隐式提交——事务内未提交的 DML 会被随后的 DDL 提交，DDL 本身不可回滚（SQL 路径与协议路径各验一次）。因此 `transactional_ddl=false`，`navidog-schema` 走现有的非事务迁移路径（逐条执行，无 BEGIN/COMMIT 包裹）。

后果：迁移中途失败时，已执行的 DDL 留在服务器上，草稿与服务器状态不一致。现有非事务路径（SQLite 重建同样非事务）已处理「失败保留草稿、提示部分执行」，DM 复用。

### 6.2 首版迁移限制与理由

首版 DM 表设计器**允许迁移，但限制为逐条执行 + 失败即停 + 明确告知不可回滚**：

- 预览页顶部固定提示：「达梦 DDL 隐式提交，迁移失败不会回滚已执行的语句」（crate locale，en/zh-CN）。
- 不做「整表重建」式迁移（`rebuilds_tables=false`）：DM 支持原地 ALTER，重建反而放大隐式提交风险。
- 不生成补偿/回滚脚本（首版）：理由——补偿需要反向 DDL 与数据回填，DM 的 IDENTITY/序列归属、STORAGE 子句使反向生成不可靠；宁可失败即停让用户手工处理，不假装能回滚。
- 外部结构变化拒绝覆盖、失败保留草稿：复用现有守卫（05-architecture-foundations）。

后续计划：M14 后续任务评估「迁移前快照 + 失败后差异报告」（只读诊断，不自动回滚）；分区/函数索引/虚拟列设计器随对应能力字段开启。

### 6.3 设计器语法

字段/约束/索引/注释的迁移语句用 5.4 的 DM 语法。加列走方言方法 `add_column`（DM `ADD (…)`），改已有列走 `modify_column`（DM 接管整段语句序列，5.4.1）。IDENTITY 列编辑：改种子/增量需重建列（DM 不支持 ALTER IDENTITY 参数），首版 IDENTITY 参数只读，改参数走「删列加列」并提示数据丢失风险。STORAGE/表空间等私有属性经高级 SQL 框编辑（`table_option_style` 无对应表单，首版 None）。

富集路径（`enriches_from_catalog=true`）：DDL 来自 `SP_TABLEDEF`（含 STORAGE 子句），表选项与列元数据经方言方法 `table_options_sql`/`column_metadata_sql`/`identity_definition` 读取（5.4.1），加载器不按产品分支。`SP_TABLEDEF` 输出解析规则阶段 2 实测核对（列定义行、约束行、STORAGE 子句边界）。

**IDENTITY 属性行只读（`identity_editable=false`）**：设计器「自增」属性行对 DM 禁用；`edit_column(Identity)` 对 `identity_editable=false` 的产品报错。创建 IDENTITY 列经定义文本框直接写 `IDENTITY(seed, increment)`。**显示限制（偏差 D10）**：属性解析器（`properties::identity`）只认 PG 的 `GENERATED … AS IDENTITY`，DM 的 `IDENTITY(seed, increment)` 定义不会被解析进 identity 字段，表单自增组显示为空（定义文本框仍显示完整子句）。把 DM IDENTITY 解析进表单列后续。

### 6.5 DefinitionFamily::Dameng 与安全工具只读列表

`navidog-sql` 的 `DefinitionFamily` 新增 `Dameng` 变体，`DamengDialect::definition_family()` 返回 `Some(Dameng)`。编译器强制所有 `match family` 补臂，语义在此登记：

- **定义编辑器**（`definition.rs`）：首版支持 View / Function / Procedure / Sequence。View/Function/Procedure 用 `CREATE OR REPLACE …`（DM 支持 OR REPLACE，阶段 2 实库回归）；Sequence 用 `CREATE SEQUENCE` / `ALTER SEQUENCE`（参数变化语法阶段 2 实测核对），改名不支持（`rename_object(Sequence)` 返回 `None`）。Index/Trigger 走表设计器的索引/触发器页，不走独立定义编辑器。
- **安全**（`security.rs`）：`account_form` → `Unavailable("Dameng user and role designers are not available yet; …")`；`account_statements`/`grant_statements` → `Err`；`account_list_sql` → `Some`（`SELECT USERNAME AS name, … FROM ALL_USERS ORDER BY USERNAME`，列布局通用，列表面板按返回列名渲染）。`manages_privileges`/`monitors_server` false（基线）。`maintenance` → 空（ANALYZE/REBUILD 等后续，F-DM-007）；`kill_sql` → `Ok(None)`、`set_variable_sql` → 空（`monitors_server=false` 时均不可达，补臂仅为穷尽）。
- **查找与依赖**：`structure_queries` → `ALL_OBJECTS WHERE OWNER = … AND OBJECT_NAME LIKE …`；`text_columns_sql` → `ALL_TAB_COLUMNS` 文本类型列（`VARCHAR`/`VARCHAR2`/`CHAR`/`CLOB`/`TEXT`）；`like_cmp` → `LIKE … ESCAPE`（DM 无 `ILIKE`，`case_insensitive_like=false`，大小写敏感随服务器 `CASE_SENSITIVE`）；`regex_sql` → `Err`（REGEXP_LIKE 后续）；`relation_plan` → note「首版不读依赖信息」（`ALL_DEPENDENCIES` 后续）；`object_exists_sql` → `ALL_OBJECTS` 存在性检查。

**安全工具只读列表（通用 UI 机制，不按产品分支）**：现有用户工具在 `account_form=Unavailable` 时直接显示原因文本（SQLite 无列表）。改为按能力驱动：`Unavailable` **且** `account_list_sql()=None` → 保持显示原因（SQLite 行为不变）；`Unavailable` **且** `account_list_sql()=Some` → 渲染只读列表（隐藏「新建」「设计」按钮、禁用双击打开，保留筛选与刷新）。`start()` 同理：`Unavailable` 但有列表 SQL 时仍拉取列表。`tools.rs::show_account` 里硬编码的 `SQLITE_AUTH_REASON` 改用 `Unavailable(reason)` 携带的原因（通用修正）。DM 的用户/角色只读导航（F-DM-009）由此复用现有列表面板。

## 7. 偏差

| 编号 | 偏差 | 理由 | 计划处理 |
| --- | --- | --- | --- |
| D1 | DDL 隐式提交，迁移不可回滚（`transactional_ddl=false`） | DM 服务器行为，实测确认 | 首版限制见 6.2；后续做失败差异报告 |
| D2 | 取消 = `SP_CLOSE_SESSION` 杀整个会话，取消后必须重连 | dm-pure 无协议级取消；DM 无「只中断语句」的等价 API（对照 Go 驱动确认） | 恢复宏按 disconnected 重连 + 事务丢失守卫；上游若加取消 API 再切换 |
| D3 | SSL 不支持（`supports_ssl=false`） | dm-pure 1.2.4 无 TLS 路径 | 上游支持后开启；ODBC 兜底路线保留在规格但不启用 |
| D4 | 建库「GBK」选项实际是 GB18030 | DM 的 UNICODE_FLAG=0 即 GB18030；dm-pure 握手返回 `gb18030` | 文案与日志按 GB18030 表述，不写 GBK |
| D5 | dm-pure 1.2.4 四处协议缺陷需补丁（LOB/EXPLAIN/错误消息/SET SCHEMA） | 上游 bug，对照官方 Go 驱动定位并验证；上游仓库已消失（404），无法 fork/提 PR | 补丁托管自建镜像 `ekovovo/dm8_pure_rs`（git 依赖）；上游恢复后提 PR，发修复版后切回 crates.io |
| D6 | 表 DDL 用 `SP_TABLEDEF`，不用 `DBMS_METADATA.GET_DDL` | GET_DDL 对 TABLE 报 -20008、其余返回空串（实测） | 视图/索引/序列 DDL 由目录合成 |
| D7 | 单数据库节点（`DAMENG`）保留在导航树 | 通用三层树（连接→数据库→模式）零分支；Navicat 达梦是否直接显示模式待 reference 核对 | 核对后如 Navicat 折叠数据库层，按能力字段调整，不按产品名分支 |
| D8 | 首版不暴露 ROWID、不做 `/` 终结符、mode 4 反斜杠转义未实测 | 控制首版范围 | 后续任务实测后开启 |
| D9 | 帮助页名暂定（reference 未导出） | 本机无 Navicat 帮助导出 | 导出后核对 features.md 帮助页列 |
| D10 | IDENTITY 属性表单显示为空 | 属性解析器只认 PG 的 `GENERATED … AS IDENTITY`，DM 的 `IDENTITY(seed, increment)` 不解析进 identity 字段（6.3）；定义文本框显示完整子句，`identity_editable=false` 禁用属性行。把 DM IDENTITY 解析进表单列后续 |

## 8. 测试与验收清单

### 8.1 测试

- 单元测试（不进 CI 的实库部分除外，全部进 CI）：方言（引用/折叠/分页/事务 SQL/能力字段随 CS 与 mode 变化）、目录 SQL 文本与参数绑定、类型映射、错误码映射、words.rs 非空且含核心保留字。
- 实库测试（**本地可选，不进 CI**——DM 镜像许可限定学习/测试/开发，1 年试用）：环境变量 `NAVIDOG_DM_LIVE=1` + `DM_PORT`（默认 15236）开关，照现有 `NAVIDOG_LIVE` 模式。覆盖：连接/错误密码、目录八方法、中文读写（UTF-8 与 GB18030 各一实例）、LOB 读写、事务与 DDL 隐式提交、取消杀会话后重连、SET SCHEMA、分页、EXPLAIN、lazy fetch、设计器迁移语法。测完 `docker compose down`（DM 容器手动起，compose 文件只含现有三库；DM 实例的起法写验收记录）。
- `navidog-headless` 增加 dameng 编译回归（可选但推荐，证明驱动栈不链接 GPUI）。
- `scripts/check-deps.sh` 名单加 `navidog-dameng`；`check-dialect-branches.sh` 必须过。

### 8.2 验收清单

- [x] 连接表单：常规/高级/SSH 页按标志显示，无数据库输入、无 SSL 页；错误密码报干净中文消息（实库测试 `-2501`；消息前缀彻底消除仍依赖补丁 3）。
- [x] 导航树目录来源：模式列表、系统模式过滤（`is_system_schema`）、表/视图/函数/过程/序列/触发器到对象级；DDL 显示 SP_TABLEDEF 原文（实库测试覆盖目录八方法）。
- [x] 表查看器：分批读取 5000 行、中文列名/数据、BLOB 十六进制显示（实库测试）；单元格编辑提交走 `navidog_db::commit` 通用路径（三方言回归），SAVEPOINT 语法阶段 0 实测。
- [x] 查询编辑器：执行/停止（停止后重连提示事务丢失）/解释文本视图（EXPLAIN 项在补丁接线前自动跳过，接线后转正）；历史日志走 `navidog-sql-log` 无方言分支。
- [x] 表设计器：迁移语法 17 条实库执行（含 IDENTITY/注释/索引/外键/检查/改表名）、迁移预览提示不可回滚（`transactional_ddl=false` 能力驱动）、IDENTITY 结构变化拒绝（plan 测试 + 实库）。
- [x] 新能力字段 `folds_unquoted_upper`/`empty_string_is_null`/`identity_editable` 与 `ExplainStyle::TextLines` 按第 5 节实现，消费者按能力分支（`typed_name`、`written_sql` 空输入写 NULL、筛选 `= ''`→`IS NULL`、解释文本视图、设计器自增行禁用），无产品名分支。
- [x] 设计器 SQL 钩子（`table_options_sql`/`column_metadata_sql`/`identity_definition`/`add_column`/`modify_column`）默认实现保持 PG 现状（PG 回归不变），DM 覆盖产出同布局结果；加载器无产品分支。
- [x] `identity_editable=false` 禁用 DM 自增属性行且 `edit_column(Identity)` 报错；IDENTITY 列结构变化提示删列重加、数据丢失。
- [x] `DefinitionFamily::Dameng` 各臂补齐；用户工具对 DM 显示只读列表（Unavailable + 列表 SQL），SQLite 原因文本行为不变。
- [x] `session_setup` 通用识别 `SET SCHEMA`（实库测试含切换与回放）。
- [ ] CS=0 与 mode 2/4 实例：标识符引用与空串行为按能力字段切换（实库本地验证，需 dm8-ci/dm8-gbk 实例，下一轮）。
- [x] 第 5 节全部检查通过（fmt/clippy/check-deps/check-dialect-branches/check-i18n/check-raw-px/全量测试）；既有方言截图无变化，DM 界面的浅深色目测随界面走查项留待下一轮。
- [x] progress.md 更新（M14 行、任务表、偏差表）；验收记录补全。
