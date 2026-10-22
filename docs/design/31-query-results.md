# M3 规格：运行、结果、解释、历史日志与命令列界面

覆盖功能：
- **运行与结果**：F-QRY-006 ~ F-QRY-009；
- **日志**：F-QRY-012、F-APP-017（SQL 日志）、F-PREF-010（日志选项）；
- **命令列界面**：F-QRY-014；
- **连接**：F-CONN-003（读取超时开始生效）。

查询编辑器本身见 [30-query-editor.md](30-query-editor.md)。

行为来源：`query.html`、`query_results.html`、`query_explain.html`、`query_parameters.html`、`log_files.html`、`console.html`、`preferences_adv.html`、`hot_keys.html`。

尺寸来源：`QueryDesignSQLEditorModule`、`SQLResultDatasetModule`、`SQLResultMessageModule`、`SQLResultSummaryModule`、`SQLResultInfoModule`、`QueryDesignParamPanel`、`QueryExplainModule`、`QueryExplainVisualModule`、`QueryExplainStatModule`、`HistoryLogViewController`、`ConsoleViewController`。

## 1. 驱动与分句（`navidog-db`）

```rust
pub struct Column {
    pub name: String,
    pub type_name: String,
    /// 结果列读的是哪张表的哪一列；服务器报告时才有
    pub origin: Option<ColumnOrigin>,
}

pub struct ColumnOrigin {
    pub schema: String,  // MySQL 的库、PostgreSQL 的模式、SQLite 的 main 或附加库
    pub table: String,
    pub column: String,
}

pub struct ResultSet {
    // M2 已有：columns、rows、affected_rows
    /// PostgreSQL 执行期间收到的 NOTICE 和 WARNING
    pub notices: Vec<String>,
}

pub trait Session {
    // M1 已有：server_version、ping、cancel_handle、close……
    /// 历史日志里的会话 ID：MySQL 的 CONNECTION_ID()、PostgreSQL 的 pg_backend_pid()；SQLite 没有
    fn session_id(&self) -> Option<u64>;
}

/// 脚本里的一条语句，以及它在原文中的字节区间
pub struct Statement {
    pub text: String,
    pub range: std::ops::Range<usize>,
}

pub fn split_script(sql: &str, dialect: Dialect) -> Vec<Statement>;
```

- **列来源**：
  - MySQL / MariaDB：文本协议的列定义里本来就有 `org_table`、`org_name` 和库名，直接填入。类型名改成 SQL 类型名（`int`、`varchar`），不再是 `MYSQL_TYPE_LONG`，这样网格的类型图标和数字右对齐对查询结果也生效。
  - PostgreSQL：现在的 `simple_query` 不带类型和来源。查询编辑器发来的单条语句先 `prepare`，取各列的类型 OID、`table_oid` 和 `column_id`，再照旧用 `simple_query` 按文本取数据；OID 对应的类型名、模式、表名和列名查目录得到，按会话缓存。`prepare` 失败（个别工具语句）时不带来源，结果只读。
  - SQLite：打开 rusqlite 的 `column_metadata` 特性，用 `column_database_name`、`column_table_name`、`column_origin_name`。
- **会话 ID**：连接时取一次存起来。
- **PostgreSQL NOTICE**：会话收集连接上收到的 NOTICE 和 WARNING，`execute` 结束时放进 `ResultSet::notices` 带回。
- **MySQL 读取超时**（F-CONN-003）：连接设置了“读取超时”时，每次 `execute` 等服务器超过这个秒数，就执行 KILL QUERY 并返回超时错误。mysql_async 没有单独的发送超时，“写入超时”仍然只保存（偏差）。
- **PostgreSQL 取消**：`cancel_handle` 改用会话自己的 TLS 设置，不再固定走明文（progress 第 4 节的遗留）。
- **分句**：新增 `split_script`，返回每条语句和它在原文中的字节区间，“运行当前语句”、错误标记、摘要页的定位都要用；`split_statements` 改为在它之上只取文本。MySQL / MariaDB 认识单独成行的 `DELIMITER <符号>`：之后用这个符号断句，`DELIMITER` 这一行本身不发给服务器。语句开头的注释留在语句里，结果选项卡的命名要用（第 4.1 节）。

## 2. 运行（F-QRY-006）

- **在哪里运行**：工具栏的“连接”列出全部连接，“数据库”列出该连接的数据库，PostgreSQL 另有“模式”。选到没打开的连接时先打开它，与在导航树里打开相同，需要密码时照常询问。
- **编辑器的会话**：查询编辑器用 `Sessions::new_session` 为自己开一个会话，第一次运行时才建立，关闭选项卡时关闭。这个会话一直保留，所以 `BEGIN`、`SET` 和临时表在多次运行之间都有效。
- **换数据库和模式**：MySQL、MariaDB 在会话上执行 `USE`；PostgreSQL 换到该数据库的新会话；SQLite 只影响代码补全和保存位置。PostgreSQL 换模式时执行 `SET search_path TO "<模式>", public`（模式本身就是 public 时只写一次），装在 public 里的扩展函数仍能直接调用。用户自己在编辑器里执行 `USE`、`SET search_path` 后，工具栏不跟着变。
- **命令**：

  | 命令 | 快捷键 | 范围 |
  | --- | --- | --- |
  | 运行 | ⌘R | 有选中文字时运行选中部分，否则运行全部 |
  | 运行当前语句 | ⇧⌘R | 光标所在的语句；光标在两条语句之间时取前一条 |
  | 运行已选择的 | | 选中部分（工具栏下拉、右键菜单） |
  | 停止 | ⌘. | 取消正在执行的语句，剩下的语句不再执行 |

  工具栏“运行 ▾”的下拉：运行、运行当前语句、运行已选择的、分隔线、遇到错误时继续（勾选项，默认不勾，存在 `settings.json`）、保留之前的查询结果（勾选项，默认不勾，存在 `settings.json`）。菜单栏“查询”里也有这两项勾选，和工具栏共用同一份设置。
- **执行过程**：先替换查询参数（第 3 节），再用 `split_script` 分句，在编辑器的会话上逐条执行。每条记下开始时间和耗时；返回结果集的语句生成结果选项卡，每条语句都写进消息页和摘要页。出错时停在这一条；勾选了“遇到错误时继续”则记下错误，接着执行下一条。出错的语句在编辑器里用错误级诊断标出，悬停显示错误信息；下次运行或修改这段文字时清除。
- 运行期间“运行”禁用、“停止”可用，编辑器仍可编辑。编辑器为空或只有注释时，提示“查询不能为空。”。每次运行先移除没固定的结果选项卡（第 4.1 节）。勾选了“保留之前的查询结果”时不移除，新的结果和解释追加在已有选项卡后面。消息、摘要、查询分析和状态仍只描述这一次运行。

## 3. 查询参数（F-QRY-009）

- 查询文本里的 `[$名称]` 是参数；字符串、引号标识符和注释里的不算。运行前按出现顺序收集不重复的名称，弹出“输入参数”（400 × 300）：每行是参数名、输入框和“原始模式”复选框，下方是“取消”“好”。
- 不勾“原始模式”时，值按方言写成字符串字面量（`Dialect::string_literal`）；勾选时原样替换，不加引号。同名参数只问一次。
- 同一个编辑器里保留上次输入的值，下次运行时预先填好。选“取消”就不运行。

## 4. 结果（F-QRY-007）

### 4.1 结果区

- **选项卡顺序**：固定的结果在最左（按固定的先后），然后是消息、摘要，再后是结果和解释（结果 1、结果 2……解释 1……）。没开“保留之前的查询结果”时，这里只有本次运行的；开着时，以前留下的排在本次新建的前面。MySQL / MariaDB 开了“显示剖析和状态”时，最后是查询分析和状态。
- **运行后显示哪一页**：由“查看 > 执行后默认显示”决定：结果/解释选项卡（默认，打开这一次新建的第一页；这次没有结果集时退回消息）、消息选项卡、摘要选项卡。
- **命名**：结果选项卡默认叫“结果 N”。语句开头紧挨着的 `-- NAME:名称` 或 `/*NAME:名称*/` 把它改成这个名称。N 按本次运行从 1 起算，留下的旧选项卡可以重名。
- **数量**：一次运行最多显示 20 个结果集，之后的结果集不建选项卡，消息页照常记录。留下的旧选项卡不占这 20 个。ponytail: 打开“保留之前的查询结果”后选项卡一直累加，全在内存里；“关闭所有结果”或逐个关闭即释放。
- **位置**：“查看 > 结果 ▸”在编辑器下方显示结果（默认）、在编辑器旁边显示结果、在新页面显示结果；结果区右上角的图标做同样的切换。选“新页面”时编辑器和结果各占整页。已经有结果时立刻停在结果页，还没有结果时留在编辑器，运行结束后再切到结果页。结果页右上角的按钮回到编辑器；停在编辑器且已有结果时，工具栏的“显示结果”回到结果页。选择存在 `settings.json`，对所有查询编辑器生效。
- **关闭按钮**：开着“保留之前的查询结果”时，结果和解释标签右侧有关闭按钮。消息、摘要、剖析、状态没有。关掉当前页就回到消息。开关关闭时不显示，仍可用右键“关闭”。
- **右键结果选项卡**：固定或取消固定、关闭、取消固定所有、关闭所有结果。这些动作由查询编辑器接收：菜单把动作发给当前焦点再往父级冒泡，焦点通常在 SQL 编辑器上，结果区不是它的父级。

### 4.2 结果选项卡

```
┌─────────────────────────────────────────────────────────────────────┐
│ 结果网格（ResultGrid）或表单视图                                       │
├─────────────────────────────────────────────────────────────────────┤
│ 单元格编辑器（打开时，与表查看器相同）                                   │
├─────────────────────────────────────────────────────────────────────┤
│ 底部栏（高 28）：单元格编辑器 数据分析 导出 固定 | SQL 或“只读” | + − ✓ ✗ | 网格 表单 │
└─────────────────────────────────────────────────────────────────────┘
```

- **网格**：`navidog-grid` 的 `ResultGrid`。选区、复制、复制为、列宽、冻结、隐藏列、查找（网格有焦点时按 ⌘F）都与表查看器相同。右键菜单不提供快速筛选：结果是一次取回的结果集，没有 WHERE 窗格。表头排序仍在已取回的行里重排。不分页，结果一次全部取回，与 Navicat 一致。ponytail: 结果全部放在内存里，几百万行时很占内存；需要时再改成流式读取。
- **底部栏**：左侧四个按钮：单元格编辑器（显示或隐藏网格下方的单元格编辑器）、数据分析（M10 之前禁用）、导出结果（M6 之前禁用）、固定。中间是产生这个结果的 SQL，单行截断，悬停显示全文；只读的结果在后面加“只读”。右侧是 + − ✓ ✗（只读时禁用）和网格视图、表单视图的切换。
- **底部状态文字**：产生结果的 SQL、批量提交的多条语句和错误信息使用同一套单行显示规则：换行在栏内折成空格，超出剩余宽度时省略，悬停保留完整原文和换行。底部栏固定 28pt 高，右侧操作按钮不被状态文字挤出。
- **固定**：固定的选项卡显示图钉图标，移到最左侧，再次运行时保留。悬停在固定的选项卡上弹出信息：时间戳、查询时间、获取时间、连接、数据库（PostgreSQL 加模式）和 SQL。再点固定按钮取消固定。
- **可编辑**：同时满足以下条件时结果可以编辑，否则只读：
  - 语句以 `SELECT` 开头（跳过开头的注释），因为刷新时要重新执行它；
  - 每一列都有来源（`Column::origin`），并且来自同一张表，同一个来源列不出现两次；
  - 这张表有主键，主键的每一列都在结果里（按 `table_columns` 的 `primary_key` 判断）。

  编辑、新增、删除和提交时机与表查看器相同：离开已修改的行、点 ✓ 或按 ⌘S 提交，✗ 放弃；走同一套 `ChangeSet` 和 `Dialect::render_changes`。提交流程从表查看器里提出来，作为 `navidog-grid` 的公开函数供两边调用。提交在该连接的元数据会话上执行（`Sessions::run_rel`），不在查询编辑器的会话上，所以不受编辑器里手动开启的事务影响，也不会提前结束它（偏差，Navicat 的做法待实测）。新增的行有没填的列，或某个值按表达式写入时，提交后在编辑器的会话上重新执行这条语句刷新结果，与表查看器“修改后同步记录”一致。

### 4.3 消息

只读文本，等宽字体，可以选中和复制。每条语句一段：

```
SELECT * FROM actor
> OK
> 时间: 0.002s

UPDATE actor SET first_name = 'A' WHERE actor_id = 1
> 受影响的行: 1
> 时间: 0.004s

SELECT * FROM nope
> 1146 - Table 'sakila.nope' doesn't exist
> 时间: 0s
```

- 错误用 `DbError` 的“代码 - 信息”格式。PostgreSQL 的 NOTICE 在 `> OK` 之前逐条写成 `> NOTICE: …`。被停止的语句写服务器返回的取消错误。
- 措辞与 Navicat 并排核对后再调整。

### 4.4 摘要

- **顶部**（高 96）：已处理的查询、成功、错误；开始时间、结束时间、运行时间；右侧是“仅显示错误”复选框。还没运行过时都显示“无”。
- **表格**：每条语句一行，列是查询、消息、运行时间。“仅显示错误”只留下出错的行。双击一行，在编辑器里选中这条语句。

### 4.5 查询分析和状态（MySQL / MariaDB）

- “查看 > 显示剖析和状态”是勾选项，默认不勾，存在 `settings.json`；PostgreSQL 和 SQLite 下禁用。
- 开启后，运行前先执行 `SET profiling = 1` 并记下 `SHOW SESSION STATUS`；运行结束后执行 `SHOW PROFILE` 和 `SHOW SESSION STATUS`。“查询分析”选项卡列出最后一条语句的各个阶段和耗时（Status、Duration）；“状态”选项卡列出运行前后有变化的状态变量和变化量（如 `Bytes_received`、`Bytes_sent`）。
- 这几条辅助语句照常写进历史日志，不写进消息页和摘要页。

## 5. 查询解释（F-QRY-008）

- **范围**：工具栏“解释”。有选中文字时解释选中的语句（按钮变成“解释已选择的”），否则解释全部语句。每条语句一个“解释 N”选项卡，可以固定。只取计划，不执行语句，不用 `ANALYZE`。
- **取计划的方式**：

  | 方言 | 语句 | 视图 |
  | --- | --- | --- |
  | MySQL 8 | `EXPLAIN`（网格）、`EXPLAIN FORMAT=JSON`（JSON、可视化、统计信息）、`EXPLAIN FORMAT=TREE`（文本） | 全部 |
  | MariaDB | `EXPLAIN`（网格；文本页显示同一张表的文字形式）、`EXPLAIN FORMAT=JSON`（JSON、可视化、统计信息） | 全部 |
  | PostgreSQL | `EXPLAIN (FORMAT JSON)`（JSON、可视化、统计信息、网格）、`EXPLAIN`（文本） | 全部 |
  | SQLite | `EXPLAIN QUERY PLAN` | 网格、文本（按 parent 缩进成树）、信息 |

- **视图切换**：选项卡顶部（高 36）：可视化、网格、文本、JSON、统计信息、信息，方言没有的不显示。JSON 和文本用只读编辑器，JSON 用 `tree-sitter-json` 高亮。PostgreSQL 的网格把计划节点展开成行：节点类型、关系、启动成本、总成本、行数、宽度，按层级缩进。“信息”页显示时间戳、查询时间、获取时间、连接、目录、数据库和 SQL。
- **可视化**：用 GPUI 的 `canvas` 自绘计划树。每个节点一个框，写操作名、关系名、耗时占比和行数；叶子在左，根在右，连线表示数据的流向。
  - 耗时占比：PostgreSQL 用节点自身的成本（`Total Cost` 减去子节点的 `Total Cost`）除以根节点的 `Total Cost`；MySQL 用表节点 `cost_info` 的 `read_cost + eval_cost` 除以 `query_cost`；MariaDB 用表节点的 `cost` 除以 `query_block` 的 `cost`。计划里没有成本字段时不显示占比，也不标红。
  - 工具栏（高 32）：耗时阈值(%)（默认 50，输入框宽 45），占比不小于阈值的节点数字变红；放大、缩小（⌘ 加滚轮也可以）；按节点类型筛选（下拉，含“清除筛选”）；查找框（宽 150，匹配节点文字并高亮）。
  - 单击节点弹出信息框（宽 171）：标题、“全部复制”和关闭按钮，下面是这个节点的全部属性（键值对）。
  - Navicat 的“下载图表为 PNG”M3 不做（偏差）。
- **统计信息**：下拉选“每个节点类型”或“每个关系”。每个节点类型的列：节点类型、计数、耗时、耗时 (%)；每个关系的列：关联名称、扫描数量、节点类型、计数、耗时、佔关系的 %。只用于 JSON 格式的计划。
- 计划解析、占比和统计是纯函数，放在 `navidog_util::explain`，用三种格式的计划样例做单元测试。

## 6. 历史日志与 SQL 日志（F-QRY-012、F-APP-017、F-PREF-010）

- **记录什么**：navidog 发给服务器的每条 SQL：查询编辑器、命令列界面、表查看器、导航树的元数据查询、对象操作。驱动里所有发出 SQL 的地方都经过同一个记录函数（包括 `list_databases` 这类不走 `execute` 的直接调用），调用方不用各自记录。
- **QueryExec.log**：写在日志路径下（默认 `~/Library/Logs/navidog/`），每次启动时清空，与 Navicat 一样只保留本次运行。每条记录：

  ```
  [2026-10-10 23:49:01.123][sakila 本地][42][MYSQL][0.002s]
  SELECT * FROM actor

  ```

  方括号依次是时间（到毫秒）、连接名称、会话 ID（SQLite 为 0）、连接类型（驱动 ID 的大写：MYSQL、MARIADB、POSTGRESQL、SQLITE）、运行时间；下面是语句，再空一行。出错时紧跟一条表头相同的记录，正文是错误信息，与 Navicat 一样。
- **写入**：在后台线程追加，界面不等待。内存里保留本次运行最近的 10,000 条，并标出哪些是错误，供查看器使用。ponytail: 超过 10,000 条时查看器只看得到最近的部分，完整内容在文件里。
- **历史日志查看器**：⇧⌘H 或“工具 > 历史日志”，在对象窗格打开一个单例选项卡。只读的 SQL 编辑器显示记录，新记录追加在末尾并滚到底；用户往上滚动后不再自动滚动，滚回底部后恢复。顶部按钮：仅显示错误（只留下出错的语句和错误信息）、暂停 / 恢复（暂停期间的新记录先攒着，恢复后一起显示）。它激活时“查看”菜单有显示日期、显示时间、显示服务器名称、显示会话 ID、显示连接类型、显示运行时间，默认全部勾选，存在 `settings.json`；只影响查看器里显示的表头，不影响文件。
- **偏好设置“高级”**：日志路径（默认 `~/Library/Logs/navidog`，“选择…”打开文件夹面板，旁边有“在 Finder 中显示”），修改后 QueryExec.log 立即换到新位置，应用日志下次启动生效；启用诊断日志（默认不勾），勾选后应用日志记录 debug 级别，否则是 info 级别。

## 7. 命令列界面（F-QRY-014）

- **打开**：⇧⌘C、“工具 > 命令列界面”、导航树中已打开连接的右键菜单。在对象窗格打开“<连接名称> - 命令列界面”选项卡，用自己的会话，数据库取工作区选择里的数据库。可以同时开多个；关闭选项卡时断开会话。它绑定在会话上，关闭连接时选项卡随之关闭，与表查看器相同（F-CONN-012）。
- **界面**：上面是只读的输出区，下面是输入区，输入区前面是提示符：MySQL、MariaDB 为 `mysql>`，PostgreSQL 为 `<数据库>=#`（超级用户）或 `<数据库>=>`，SQLite 为 `sqlite>`；语句还没结束时换成续行提示符（`    ->`、`<数据库>-#`、`   ...>`）。字体与查询编辑器相同，没有行号。Navicat 是一整块终端式文本（偏差）。
- **输入**：回车时，如果输入以语句结束符（`;`，或 `DELIMITER` 设的符号）结尾就执行，否则换行继续输入。↑、↓ 翻看本选项卡的输入历史。⌘. 取消正在执行的语句。`exit`、`quit` 关闭选项卡。不支持 `\d`、`\c`、`.tables` 这类客户端命令（偏差）。
- **输出**：MySQL、MariaDB、SQLite 用 mysql 客户端的样式，PostgreSQL 用 psql 的样式：

  ```
  mysql> SELECT actor_id, first_name FROM actor LIMIT 1;
  +----------+------------+
  | actor_id | first_name |
  +----------+------------+
  |        1 | PENELOPE   |
  +----------+------------+
  1 row in set (0.00 sec)

  sakila=# SELECT actor_id, first_name FROM actor LIMIT 1;
   actor_id | first_name
  ----------+------------
          1 | PENELOPE
  (1 row)
  ```

  不返回结果集的语句写 `Query OK, 1 row affected (0.01 sec)`，错误写 `ERROR 1146: Table 'sakila.nope' doesn't exist`（PostgreSQL 是 `ERROR:  relation "nope" does not exist`）。数字列右对齐，NULL 显示为 `NULL`。列宽、补空格和表头居中统一按 Unicode 显示宽度计算，覆盖中日韩宽字符、全角字符和组合字符。SQLite 不用 sqlite3 默认的竖线列表，因为不便阅读。表格格式化是纯函数，放在 `navidog-util`。
- 输出区最多保留最近 10,000 行。ponytail: 更早的输出丢弃；需要时再加“保存输出”。每条语句同样写进历史日志。

## 8. 菜单与快捷键

| 菜单 | 菜单项 |
| --- | --- |
| 查询 | 只在查询编辑器激活时出现（10 第 9 节）。运行（⌘R）、运行当前语句（⇧⌘R）、停止（⌘.）、解释、遇到错误时继续（勾选）、保留之前的查询结果（勾选） |
| 工具 | 命令列界面（⇧⌘C）、历史日志（⇧⌘H） |
| 查看 | 结果 ▸ 在编辑器下方显示结果、在编辑器旁边显示结果、在新页面显示结果；执行后默认显示 ▸ 结果/解释选项卡、消息选项卡、摘要选项卡；显示剖析和状态；历史日志激活时还有显示日期……显示运行时间 |

“工具”菜单在 M3 加入菜单栏。查询编辑器的 key context 里：⌘R 运行，⇧⌘R 运行当前语句，⌘. 停止。

## 9. 新增 token（登记到 02-design-tokens.md）

| token | 值 | 来源 |
| --- | --- | --- |
| `result.bottom_bar.height` | 28 | nib:SQLResultDatasetModule/底部栏（811 × 28） |
| `result.bottom_bar.button` | 20 | nib:SQLResultDatasetModule/NSButton（20 × 20，间距 28） |
| `result.summary.height` | 96 | nib:SQLResultSummaryModule/概况区（663 × 96） |
| `params.window` | 400 × 300 | nib:QueryDesignParamPanel/NSWindowRect |
| `explain.view_switch.height` | 36 | nib:QueryExplainModule/视图切换（247 × 36） |
| `explain.toolbar.height` | 32 | nib:QueryExplainVisualModule/工具栏（513 × 32） |
| `explain.threshold.width` | 45 | nib:QueryExplainVisualModule/NSTextField（45 × 24） |
| `explain.search.width` | 150 | nib:QueryExplainVisualModule/NSSearchField（150 × 24） |
| `explain.node_info.width` | 171 | nib:QueryExplainVisualModule/节点信息框（171 × 212，标题栏高 30） |
| `explain.node_info.height` | 212 | 同上，信息框的最大高度 |
| `result.tab_height` | 26 | 暂定：结果区选项卡条 |
| `explain.node` | 160 × 56 | 暂定，待并排目测 |
| `explain.node_gap` | 40 × 16 | 暂定，横向是层与层的间距，纵向是兄弟节点的间距 |

## 10. 偏差

- MySQL 的“写入超时”仍然只保存：mysql_async 没有单独的发送超时。
- 查询结果的修改在元数据会话上提交，不参与编辑器里手动开启的事务。
- 可视化解释不能下载为 PNG。
- 命令列界面分成输出区和输入区；不支持客户端命令；SQLite 用 mysql 的表格样式；PostgreSQL 不返回结果集的语句也写 `Query OK`，不写 psql 的命令标签。
- QueryExec.log 的连接类型用驱动名称的大写（MYSQL、MARIADB、POSTGRESQL、SQLITE）。
- 查询结果的表单视图只读，用上一条、下一条浏览；修改用网格或单元格编辑器。表头排序对已取回的行在本地排序。
- 历史日志查看器每 250ms 检查一次新记录。
- 命令列界面的提示符在输入框左侧，输入有多行时换成续行提示符；输出区的回显照客户端的样子逐行带提示符。

## 11. 验收清单

- [x] 多行 SQL、批量提交状态和长文本在浅色及深色的底部栏中保持单行，操作按钮不被挤出，悬停保留原文。（复用 `skin::truncated`，`batch_sql_stays_on_one_line_beside_footer_controls` 验证几何约束；查询结果截图及共用组件的悬停截图见 [验收记录](../acceptance-status-overflow.md)。）
- [x] 三种数据库：运行、运行已选择的、运行当前语句、停止都正确；多条语句各自写进消息和摘要；出错时停下并标出语句，勾选“遇到错误时继续”时执行完全部。（SQLite：`runs_scripts_into_messages_summary_and_editable_results`；MySQL、MariaDB、PostgreSQL：`NAVIDOG_LIVE=1 cargo test -p navidog-query live_editor`。停止前先确认服务端正在执行慢查询。）
- [x] 用 `DELIMITER $$` 创建存储过程后能调用；`-- NAME:` 改了结果选项卡的名称；超过 20 个结果集时只显示 20 个。（MySQL、MariaDB 的 `live_editor_runs_selection_current_errors_and_stop` 创建、调用、删除测试过程并验证结果名称；SQLite 编辑器测试验证 21 条均执行但只保留 20 个结果。）
- [x] 查询参数：同名参数只问一次，原始模式不加引号，上次输入的值预先填好。
- [x] 结果编辑：`SELECT * FROM actor` 可以编辑并提交；联接多表的结果、没有主键的表、`CALL` 返回的结果都显示“只读”。
- [x] 固定的结果在再次运行后保留，悬停显示信息。（再次运行见查询编辑器测试；悬停文案是 `RunInfo::lines`，见 `pinned_hover_lists_when_and_where`）
- [x] 勾选“保留之前的查询结果”后，再次运行留下上次的结果，当前页是这次新建的；消息和摘要只记这一次。标签上的关闭按钮能关掉一页。焦点在 SQL 编辑器时，右键的固定动作仍然生效。
- [ ] 解释：MySQL 8、MariaDB、PostgreSQL、SQLite 的各个视图都正确；调低耗时阈值后节点数字变红；统计信息的耗时合计等于根节点的成本。
- [x] 历史日志查看器实时显示所有语句；仅显示错误、暂停和恢复、表头开关都生效；QueryExec.log 的格式正确，重启后清空。
- [x] 命令列界面能执行多行语句、续行、翻历史、取消；输出的表格对齐。（`history_log_follows_the_log_and_the_console_runs_statements` 通过真实按键验证续行、历史、取消和取消后再次执行；`unicode_columns_align_by_display_width` 覆盖中日韩、全角和组合字符的列宽。）
- [x] PostgreSQL 开 SSL（require）时，“停止”能取消正在执行的语句。
- [x] MySQL 设置读取超时 2 秒后，`SELECT SLEEP(5)` 被取消并报告超时。
- [ ] 消息页的措辞与 Navicat 并排核对。
