# PR #31 审查：M4 剩余工作

- 仓库：`xhofe/navidog`
- PR：<https://github.com/xhofe/navidog/pull/31>
- 基准：`main @ 43dd4c6`
- 头部：审查时 `feat/m4-remainder @ 05a1f78`。第 7.1–7.3 节的修复在 `9dccbb6`。其后的补充见第 7.4 节，与该节的文档更新在同一提交。
- 范围：`git diff main...HEAD`，51 个文件，+7857 / −204
- 审查维度：功能实现、代码设计、实现逻辑、UI 设计
- 审查方式：读规格与 diff、跑静态检查和定向测试、用 Docker 实库复现关键 SQL、查看浅色截图
- 结论：**不建议按当前状态合并（Request changes）**。表设计器的 PostgreSQL 整表重建存在静默丢触发器的问题；对象设计器、用户/权限、监控、查找有较多“有界面但不符合规格”的实现，且 `progress.md` 把它们标成了“完成”。

---

## 0. 结论摘要

### 0.1 做得好的地方

- **依赖方向清楚**：SQL 计划在 `navidog-sql`/`navidog-schema`，界面在 `navidog-designer`，外壳只负责打开面板；没有新增 feature crate 之间的依赖，`scripts/check-deps.sh` 通过。
- **表设计器计划测试有基础**：排除约束、identity、SQLite 生成列/`WITHOUT ROWID`、列重排都有计划级测试；MySQL/PostgreSQL/SQLite 的实库建表测试也在。
- **复用到位**：`FormRow`、`tokens`、`tabs`、`Sessions::new_session`、`run_rel` 都沿用了现有模式；新代码没有裸像素，`scripts/check-raw-px.sh` 通过。
- **i18n 基础设施正确**：新增 key 同时有 `en`/`zh-CN`，动态 key 已登记，`scripts/check-i18n.sh` 通过。
- **对象设计器接入了选项卡**：关闭守卫、脏标记、预览/执行分离、移动窗口的基本框架都在；表设计器的 `on_removed` 已经正确处理“移动到新窗口”。

### 0.2 阻断合并的问题

| 级别 | 问题 | 位置 |
| --- | --- | --- |
| **P0** | PostgreSQL 列重排重建会静默丢掉已有触发器 | `table_design/mod.rs:316-334`、`plan.rs:643` |
| **P1** | PostgreSQL 重建在命名 PK/UNIQUE 约束和 `serial` 列上直接失败 | `table_design/plan.rs:597-643`、`mod.rs:113-140` |
| **P1** | 对象设计器把已有对象一律当 raw DDL，选项页被隐藏；新建视图/函数等有服务器 DDL 的对象，第二次保存会被外部变更守卫拒绝 | `designer/object.rs:291-336`、`:272`、`:583`、`schema/object_design.rs:21-31` |
| **P1** | 把选项卡移到新窗口会绕过关闭确认；工具面板还会取消监控任务 | `designer/object.rs:535-539`、`designer/tools.rs:697-704` |
| **P1** | 用户面板不能选择/编辑已有账号，保存已有用户会走 `CREATE USER`；密码输入未遮挡 | `designer/tools.rs:205-227`、`:358-365`、`:820-825` |
| **P1** | 查找结果只是文本，不能双击打开；SQLite 正则运行时报错；MySQL 区分大小写在 latin1 连接上失败 | `designer/tools.rs:530-573`、`sql/security.rs:955`、`:1008` |
| **P1** | 触发器快照只对 SQLite 加载，PostgreSQL/MySQL 的已有触发器不会出现在触发器页 | `table_design/mod.rs:316-334` |
| **P1** | MySQL 分区表单忽略分区名，生成的 SQL 缺少 `PARTITION p0` | `designer/panel/more.rs:196-203` |
| **P1** | PostgreSQL 监控的“结束”用的是 `pg_terminate_backend`，且变量页允许 `ALTER SYSTEM` 写入 | `sql/security.rs:343`、`:389-394` |

### 0.3 合并建议

1. **先修 P0/P1**，并为每个修复补回归测试；尤其要补一个“真实 PostgreSQL 快照”的重建测试，而不是手工往 `TableSnapshot.triggers` 里塞触发器。
2. **把 `progress.md` 的状态改成“部分”**：F-DSG-005/006/008、F-DSG-010 至 F-DSG-019、F-INFO-003/008、F-SEC-001 至 F-SEC-004、F-TOOL-001/002 目前都标成“完成”，但对应验收项仍未勾选，且多处 UI 只是最小实现。其中 F-DSG-010 的“完成”尤其不准确：PostgreSQL 列重排的 P0/P1 问题就在这条功能的验收范围里。
3. 如果时间有限，建议拆成两个 PR：
   - PR A：表设计器的排除/触发器/分区 + PostgreSQL 重建修复（先修 P0/P1）；
   - PR B：对象设计器、用户/权限、监控、查找（把未完成项明确标为部分）。
4. 合并前至少补：PostgreSQL 命名约束/serial/触发器重建、对象设计器二次保存、移动到新窗口、已有用户编辑、SQLite 正则、MySQL latin1 查找的测试。

---

## 1. 功能实现

### 1.1 规格对照

| 规格 | 状态 | 说明 |
| --- | --- | --- |
| 41 §1 排除约束 | 基本可用 | 表单和 SQL 计划都在，但只有计划测试，没有 PostgreSQL 实库往返 |
| 41 §2 触发器 | **部分** | PG/MySQL 快照不加载已有触发器；表单硬编码 `FOR EACH ROW`，没有事件多选和 statement 级 |
| 41 §3 分区 | **部分** | MySQL 分区名被忽略；PG 已有子分区不加载、不能删除 |
| 41 §4 迁移 | **部分** | PG 列重排在常见表上失败或丢触发器；测试未覆盖真实快照 |
| 42 §1 共用面板 | 基本可用 | 选项卡、保存/预览/执行、关闭守卫都在；移动到新窗口的守卫有 bug |
| 42 §2 视图 | **部分** | 已有视图加载成整段 `CREATE VIEW`，定义页不是 `SELECT`，选项页被隐藏 |
| 42 §3 函数/过程 | **部分** | 参数是一个自由文本选项，不是规格要求的参数网格；运行对话框只有一个文本框 |
| 42 §4 事件 | 基本可用 | 计划生成；已有事件仍是 raw DDL |
| 42 §5 数据库/模式 | **部分** | 已有库/模式不加载属性；选项改动不标记 dirty，关闭会静默丢失 |
| 42 §6 其他对象 | **部分** | 扩展是“action”文本框，不是可用/已安装双列表；索引/触发器已有对象是 raw DDL |
| 43 §1 用户与角色 | **部分** | 没有账号列表选择/编辑；缺插件、SSL、确认密码、成员、服务器/对象权限网格；密码未遮挡 |
| 43 §2 权限管理员 | **部分** | 自由文本表单，不是“对象树 + 用户/角色 + 每权限一个复选框” |
| 43 §3 维护 | 基本可用 | 维护面板和命令清单在；结果格式复用不足 |
| 43 §4 信息窗格 | **部分** | 使用/被使用是纯文本，不能点击；Members 页不可达；MySQL 只读外键，不读视图/例程/触发器定义 |
| 43 §5 服务器监控 | **部分** | 三页只是按钮、没有选中态；PG 结束进程语义错；PG 变量可写 |
| 43 §6 在库中查找 | **部分** | 没有连接/库/模式选择器；结果不能双击；SQLite 正则和 MySQL latin1 会报错 |

### 1.2 功能层面的主要偏差

1. **“完成”状态与验收清单不一致。** `docs/pr/acceptance-m4-remainder.md` 明确说排除/触发器/分区往返、视图替换、函数运行、用户创建、依赖页、结束进程、查找双击等验收项仍未勾选；但 `docs/progress.md` 把对应功能标成“完成”。这不是简单的“没跑实库”，而是部分 UI 本身没有实现规格要求的交互。建议把状态改为“部分”，并在偏差表里写清“raw DDL”“自由文本权限表单”“文本查找结果”等。

2. **对象设计器的“已有对象”路径是退化的。** 规格 42 §2 说视图定义是一条 `SELECT`，§3 说函数/过程用参数网格；实现把所有 `catalog_kind()` 非空的对象都加载成整段服务器 DDL，并设置 `raw = true`，于是：
   - 已有 MySQL 视图看不到算法/安全性/检查选项；
   - 已有函数看不到参数/返回类型/语言；
   - 已有索引/触发器看不到表名和结构化定义；
   - 第二次保存被外部变更守卫拒绝（见 3.3）。
   这不是“解析不了的已有定义保留原文”那条例外，而是把例外当成了默认路径。

3. **用户/权限只是最小表单。** 规格 43 §1 要求插件、SSL、确认密码、成员属于、服务器权限、对象权限；实现只有名称、主机、密码、锁定、过期、资源限制和 PostgreSQL 的几个布尔项。权限管理员规格要求对象树和复选网格，实现是“权限/对象/被授予者”三个文本框。验收项未勾选是合理的，但状态不应标“完成”。

4. **查找没有结果浏览器。** 规格要求连接/库/模式选择器、结构/数据范围、每对象最多 100 行、结果双击打开设计器或表查看器。实现把结果拼成一个字符串，放在 `tool-output` 里逐行渲染；没有行模型，也没有双击。即使 SQL 能跑，核心交互也没有。

5. **监控的“三页”没有页面状态。** 进程/变量/状态是三个按钮，共用一块输出；没有选中态，自动刷新永远重新加载进程页。规格说的是三页，用户应该知道当前在哪一页。

---

## 2. 代码设计

### 2.1 边界与分层

好的部分：

- `navidog-sql` 负责方言 SQL 和计划，`navidog-schema` 负责执行顺序，`navidog-designer` 只做界面；`navidog` 外壳按对象种类打开面板。这个方向与 00-overview 的 crate 表一致。
- 新代码没有把驱动或 GPUI 带进 `navidog-sql`/`navidog-schema`；`check-deps.sh` 通过。
- 对象设计器复用一个面板，而不是给每种对象写一个面板；这符合 42 §1 的“一个面板”。

### 2.2 设计问题

#### 2.2.1 `DefinitionFamily` 仍是产品分支

`crates/navidog-sql/src/definition.rs:13-16` 定义了 `Mysql / Postgres / Sqlite` 三值枚举，`security.rs` 里有 50 处 `DefinitionFamily::…` 匹配。虽然它没有直接比较 `Dialect::id()`，所以 `scripts/check-dialect-branches.sh` 放行，但本质仍是“按产品名分支”。新增第四个产品时，需要在 `security.rs` 的几十个 match 里逐项补，容易漏掉某个 SQL 细节。

建议：

- 把安全/监控/查找的 SQL 拆成方言方法（或每个驱动一个 `security` 模块），让 `navidog-sql` 只保留通用 trait；
- 或者至少把 `DefinitionFamily` 收敛成少量能力字段（`account_form`、`kill_style`、`find_case_insensitive`……），不要让一个 50 臂的 match 成为事实上的产品开关；
- 如果保留现状，在 00-overview 或 ADR 里明确这是“受控的产品分支”，并给 `check-dialect-branches.sh` 加一条对 `DefinitionFamily` 的提示，而不是让它静默通过。

#### 2.2.2 UI 用无关的 feature flag 推断产品

`crates/navidog-navigator/src/navigator/menus.rs:165-167`：

```rust
let mysql = features.is_some_and(|f| f.inline_table_options);
let postgres = features.is_some_and(|f| f.has_schemas);
let sqlite = features.is_some_and(|f| f.rebuilds_tables);
```

这三个字段的语义分别是“表选项内联”“有模式”“重建表”，不是“是不是 MySQL/PostgreSQL/SQLite”。一个新方言只要有 schema 就会被当成 PostgreSQL，从而启用错误的“新建数据库”菜单。建议增加明确的 UI 能力方法，例如 `supports_database_designer()`、`supports_schema_designer()`，或直接根据 `object_kinds()`/`DefinitionKind` 的支持情况决定菜单。

#### 2.2.3 `raw` 与 `loaded` 的语义混在一起

`ObjectDesigner` 用 `raw: bool` 表示“body 是整段服务器 DDL”，同时用 `loaded: String` 表示“上次加载/保存的基线”。这两个字段在结构化对象上语义不同：

- 对结构化对象，`body` 是 `SELECT`/函数体，`loaded` 应该是服务器 DDL 或一个修订号；
- 对 raw 对象，`body` 和 `loaded` 都可能是 DDL。

现在保存后把 `loaded = body`（`object.rs:272`），守卫却拿服务器 DDL 和 `loaded` 比（`object_design.rs:21-31`），于是“苹果和橘子”比较。建议把状态拆成：

```rust
enum DefinitionSource {
    Structured { body: String, options: BTreeMap<String, String> },
    RawDdl { ddl: String },
}
struct Baseline { server_ddl: String, saved_body: String, revision: u64 }
```

至少要把 `loaded_ddl` 和 `body` 分开，保存后重新拉取服务器 DDL 作为基线。

#### 2.2.4 关闭守卫重复，移动窗口的语义不一致

`ObjectDesigner`、`ToolPanel`、`TableDesigner` 各自实现 `ask_close`/`on_removed`。`TableDesigner` 在 `panel.rs:383-389` 正确处理了“`tabs::removed` 为 true 时直接 return”，另外两个没有。这是典型的“复制粘贴后漏改一处”。建议抽一个共享 helper：

```rust
fn on_removed(panel: PanelId, cx: &mut Context<...>) -> bool {
    if tabs::removed(panel, cx) { return true; } // moved: keep state
    // closed: cancel tasks
    false
}
```

#### 2.2.5 `ToolPanel` 是一个 god object

`crates/navidog-designer/src/tools.rs:75` 的 `ToolPanel` 同时持有用户、权限、监控、查找四个工具的状态，约 1300 行，`render` 里按 `Tool` 分派。M4 可以接受，但继续加工具会失控。建议至少拆成 `UsersPanel`、`PrivilegesPanel`、`MonitorPanel`、`FindPanel` 四个子模块，共享一个 `ToolPanel` 外壳；或者先按 `impl` 分文件，降低单文件复杂度。

#### 2.2.6 结果格式化重复三份

- `designer/tools.rs:1245` `format_sets`
- `navigator/ops.rs:357` `format_maintenance`
- `navigator/info.rs:307-333` 关系查询的内联格式化

三份逻辑相似但不完全一致（空结果、affected rows、标签）。建议抽一个 `navidog_util::format::result_sets(sets, options)`，把“空集显示什么”“是否带标签”“是否显示 affected rows”作为参数。否则以后修一个格式，另外两处会漂移。

### 2.3 测试设计

- `postgres_column_reorder_rebuilds_without_cascade`（`table_design/tests.rs:967-993`）手工把触发器塞进 `TableSnapshot.triggers`，但真实快照在 PostgreSQL 上根本不会填这个字段。这个测试给了“触发器会被重建”的假安全感。
- `definition_plans_cover_each_object_kind` 只验证 SQL 字符串，不验证 `ObjectDesigner::load` → `draft` → `save` 的路径；而 raw 模式恰好绕过了结构化路径。
- `security_statements_cover_each_product` 只验证 SQL 字符串，不验证 SQL 在真实服务器上可执行；MySQL `utf8mb4_bin` 和 SQLite `REGEXP` 都属于“字符串测试通过、运行时失败”。
- 新增的排除/触发器/分区页、权限/监控/查找面板没有 headless UI 测试，也没有截图；本 PR 新增的截图只有对象设计器和用户面板。

建议补的测试见第 5 节。

---

## 3. 实现逻辑

### 3.1 PostgreSQL 整表重建：命名约束和 serial 会失败

**位置**：`crates/navidog-schema/src/table_design/plan.rs:597-643`、`mod.rs:113-140`

`postgres_rebuild` 先用临时名生成临时表：

```rust
let temp = format!("{}__navidog_new", old.name);
let mut out = vec![rebuild_note(after.create_sql(d, scope, &temp)?, false)];
```

`create_sql` 把 `self.constraints` 原样写进 `CREATE TABLE`（`mod.rs:120`）。PostgreSQL 的 PRIMARY KEY/UNIQUE 约束会创建同名索引，而索引名是 schema 级的；旧表还占着 `zz_named_pkey`，临时表再建同名约束就会失败。

本次用 Docker 实库复现：

```text
CREATE TABLE zz_named (id int, CONSTRAINT zz_named_pkey PRIMARY KEY (id));
CREATE TABLE zz_named__navidog_new (id int, CONSTRAINT zz_named_pkey PRIMARY KEY (id));
ERROR:  relation "zz_named_pkey" already exists
```

`serial` 列同理：从目录读回的列定义是 `integer NOT NULL DEFAULT nextval('zz_s_id_seq'::regclass)`，临时表复制了这个默认值，于是临时表的默认值依赖旧表的序列；`DROP TABLE zz_s` 被这个内部依赖挡住：

```text
CREATE TABLE zz_s (id serial PRIMARY KEY);
CREATE TABLE zz_s__navidog_new (id integer NOT NULL DEFAULT nextval('zz_s_id_seq'::regclass));
DROP TABLE zz_s;
ERROR:  cannot drop table zz_s because other objects depend on it
DETAIL:  default value for column id of table zz_s__navidog_new depends on sequence zz_s_id_seq
HINT:  Use DROP ... CASCADE to drop the dependent objects too.
```

这不是规格里说的“入站外键或视图挡住 DROP”，而是计划自己制造出来的依赖。事务会回滚，数据不会丢，但“PostgreSQL 现有列重排”在常见表上不可用；`progress.md` 把它标成完成、验收清单打勾是不准确的。

**建议**：

- 临时表的 PK/UNIQUE 约束用临时名（例如 `__navidog_new_pkey`），`DROP TABLE` 旧表后再 `ALTER TABLE … RENAME CONSTRAINT` 改回原名；或者临时表先不带命名约束，重命名后再 `ADD CONSTRAINT`。
- `serial`/`nextval` 默认值：为临时表创建新序列，或在 `DROP TABLE` 前先 `ALTER TABLE temp ALTER COLUMN id DROP DEFAULT`，重命名后再恢复默认值并修正序列的 `OWNED BY`。要同时处理 `bigserial` 和显式 `nextval`。
- 加测试：命名 PK、命名 UNIQUE、`serial`、`bigserial`、identity、普通列各一条；测试应从真实快照构造，而不是只测 `create_sql` 字符串。

### 3.2 PostgreSQL 列重排会静默丢触发器

**位置**：`crates/navidog-schema/src/table_design/mod.rs:316-334`、`plan.rs:643`

快照只在 `dialect.features().rebuilds_tables`（SQLite）时查询 `sqlite_schema` 的触发器：

```rust
let mut triggers = Vec::new();
if dialect.features().rebuilds_tables {
    // SELECT ... FROM sqlite_schema WHERE type IN ('index','trigger')
    design.triggers = Some(triggers.clone());
}
```

PostgreSQL/MySQL 的 `TableSnapshot.triggers` 永远是空 `Vec`，`TableDesign.triggers` 是 `None`。`postgres_rebuild` 却调用：

```rust
out.extend(resolved_triggers(after, Some(before)));
```

`resolved_triggers` 在 `after.triggers` 为 `None` 时回退到 `before.triggers`，而 `before.triggers` 在真实 PG 快照里是空的。于是重建后的新表没有触发器，旧表被 `DROP`，触发器静默消失。测试之所以通过，是因为它手工写了：

```rust
triggers: vec!["CREATE TRIGGER audit AFTER INSERT ON t EXECUTE FUNCTION log()".into()],
```

**影响**：任何带触发器的 PostgreSQL 表，只要用户调整列顺序，触发器就会丢。这是 schema 级数据丢失，标 P0。

**建议**：

- 在 `snapshot` 里为所有支持触发器的方言加载触发器（PG 用 `pg_trigger`/`pg_get_triggerdef`，MySQL 用 `information_schema.TRIGGERS`），或者至少 PG/MySQL 也填 `TableSnapshot.triggers`；
- 在重建前检查 `before.triggers` 非空且无法重建时，明确拒绝并提示，而不是静默丢；
- 用真实快照测试：建表 → 建触发器 → 读快照 → 重排 → 执行 → 断言触发器仍在。

### 3.3 对象设计器：已有对象 raw 化 + 第二次保存失败

**位置**：`crates/navidog-designer/src/object.rs:291-336`、`:272`、`:583`，`crates/navidog-schema/src/object_design.rs:21-31`

`load()` 对任何 `catalog_kind()` 非空的对象都做：

```rust
this.loaded = ddl.clone();
this.raw = true;
this.body.update(cx, |s, cx| s.set_value(ddl, window, cx));
```

这带来两个问题。

**问题一：选项页被隐藏。**

```rust
.filter(|(page, _)| *page != Page::Options || !self.options.is_empty() && !self.raw)
```

已有 MySQL 视图的算法/安全性/检查选项、已有函数的参数/返回类型/语言、已有索引/触发器的表名，都不可见。规格 42 §2/§3 要求这些是结构化选项；只有“解析不了的已有定义”才整段放定义框（42 §6 第 59 行）。

**问题二：第二次保存被守卫拒绝。**

保存成功后：

```rust
this.previous = Some(this.name.read(cx).value().trim().to_string());
this.loaded = this.body.read(cx).value().to_string();
```

`loaded` 变成了编辑器里的 `body`。下一次保存时，`object_design::apply` 会：

```rust
let current = r.object_ddl(scope, kind, prev).await.unwrap_or_default();
if !current.is_empty() && current != draft.loaded {
    return Err(DbError::msg(
        "The object changed outside this designer. Reload before saving.",
    ));
}
```

对新视图，`body` 是 `SELECT …`，`object_ddl` 返回 `CREATE VIEW … AS SELECT …`，两者永远不相等；对编辑过的已有 raw 对象，服务器 DDL 的格式/规范化也可能和用户文本不同。结果是：**第一次保存成功，下一次保存就会报“对象在外部被修改”**（新视图是必然，已有 raw 对象取决于服务器是否原样回读）。提交 `05a1f78` 的标题是 “fix: keep later saves and find queries executable”，但这条路径并没有真正修好。

**建议**：

- 保存成功后重新调用 `object_ddl`，把返回值写入 `loaded`；如果是新建对象，保存后也拉一次服务器 DDL。
- 对结构化对象，守卫比较的应该是“上次服务器 DDL”与“当前服务器 DDL”，而不是 `body`。
- 对已有视图/函数，先尝试结构化加载（视图取 `pg_get_viewdef`/`SHOW CREATE VIEW` 的 `AS` 部分，函数取参数和函数体）；解析失败才进入 raw 模式。
- 加 headless/实库测试：新建视图 → 保存 → 再改一次 → 再保存；已有视图 → 改定义 → 保存 → 再保存。

### 3.4 移动到新窗口会绕过关闭确认

**位置**：`crates/navidog-designer/src/object.rs:535-539`、`crates/navidog-designer/src/tools.rs:697-704`、对照 `crates/navidog-designer/src/panel.rs:383-389`

`tabs::removed` 在“移动到新窗口”时也返回 true。`TableDesigner` 的处理是：

```rust
if tabs::removed(PanelId::from(cx.entity_id()), cx) {
    return;
}
self.task.take();
```

`ObjectDesigner` 却：

```rust
if tabs::removed(PanelId::from(cx.entity_id()), cx) {
    self.close_approved = true;
}
```

`ToolPanel` 更严重：

```rust
self.auto = false;
self.watch.take();
self.task.take();
if tabs::removed(...) {
    self.close_approved = true;
}
```

后果：

- 对象设计器移动到新窗口后，`close_approved = true`，再关闭不会询问未保存修改；
- 工具面板移动到新窗口后，自动刷新停止、正在跑的查询被取消，用户以为只是换了个窗口。

**建议**：按 `TableDesigner` 的模式，`tabs::removed` 为 true 时直接 return，不设置 `close_approved`，也不取消任务；只有真正关闭时才清理。

### 3.5 用户面板：不能编辑已有账号

**位置**：`crates/navidog-designer/src/tools.rs:205-227`、`:358-365`，`crates/navidog-sql/src/security.rs:617-671`

`account_draft` 构造 `AccountDraft` 时用了 `..AccountDraft::default()`，没有填 `previous_name`/`previous_host`。`mysql_account` 用 `previous_name` 判断是 `ALTER USER` 还是 `CREATE USER`：

```rust
if let Some(previous) = previous {
    // RENAME / ALTER
} else {
    sql.push(format!("CREATE USER {user}{}", alter.trim_end()));
}
```

“列出用户”只是执行 `SELECT User, Host FROM mysql.user` 并把结果打到输出区，没有行选择，也没有把选中行填回表单。因此：

- 用户无法在面板里打开一个已有用户；
- 如果手动输入已有用户名再保存，会生成 `CREATE USER`，服务器报“用户已存在”；
- `previous_host` 永远为空，重命名/改主机的路径也不可达。

**建议**：

- 把“列出用户”改成可选择的结果列表（或下拉），选中后填充名称/主机并设置 `previous_name`/`previous_host`；
- 保存时根据 `previous_*` 生成 `ALTER USER`/`RENAME USER`；
- 密码输入加遮挡，并补确认密码；
- 补插件、SSL、成员属于、服务器权限、对象权限；至少把未实现项在 UI 上明确标出，不要标“完成”。

### 3.6 查找：结果模型和 SQL 方言都有问题

**位置**：`crates/navidog-designer/src/tools.rs:530-573`、`:1147-1207`，`crates/navidog-sql/src/security.rs:904-1024`

**结果模型**：`find` 把结构结果和数据结果都拼进 `String`，`render_output` 逐行显示。没有 `(object, kind, row)` 模型，所以无法双击打开设计器/表查看器，也无法显示列名、对象类型、命中行号。规格 43 §6 的核心交互缺失。

**SQLite 正则**：`predicate` 对 SQLite 生成：

```rust
(DefinitionFamily::Mysql, false) | (DefinitionFamily::Sqlite, _) => {
    format!("{column} REGEXP {literal}")
}
```

`navidog-sqlite/Cargo.toml` 只开了 `bundled`/`column_decltype`/`column_metadata`，没有 `functions`；源码里也没有 `create_scalar_function("regexp", …)`。bundled SQLite 默认不提供 `regexp()`，这条 SQL 会调用一个未注册的函数，运行时报 `no such function: REGEXP`。

**MySQL 区分大小写**：生成：

```rust
format!("{column} LIKE {literal} COLLATE utf8mb4_bin ESCAPE {escape}")
```

`COLLATE utf8mb4_bin` 要求连接/列的字符集是 utf8mb4。连接使用 latin1 时（Navicat 允许），实库报：

```text
ERROR 1253 (42000): COLLATION 'utf8mb4_bin' is not valid for CHARACTER SET 'latin1'
```

用 `--default-character-set=utf8mb4` 才成功。建议用 `BINARY`/`CAST(... AS BINARY)` 做大小写敏感比较，不要绑定 utf8mb4。

**结构范围缺少 Index/Trigger**：`find` 的种类来自 `dialect.object_kinds()`；PostgreSQL 的列表（`postgres/dialect.rs:33-49`）和 MySQL 的列表（`mysql/dialect.rs:29-37`）都没有 `Index`/`Trigger`，所以结构范围无法选索引/触发器。规格 43 §6 要求“当前库有的种类”。

**SQLite 数据范围漏视图**：`text_columns_sql` 的 SQLite 分支：

```sql
WHERE m.type = 'table' AND (...)
```

规格说数据范围查表和视图的文本列；这里只查表。

**建议**：

- 引入结构化的查找结果行，至少包含 `scope/kind/name/column/row/ preview`；结果表支持双击。
- 加连接/库/模式选择器，或者明确“只查当前选择”，并在 UI 上写清楚。
- SQLite 注册 `regexp`（开 `rusqlite` 的 `functions`，用 `regex` crate），或者对 SQLite 改用 `GLOB`/`LIKE` 并明确限制。
- MySQL 大小写敏感用 `BINARY`，不要用 `COLLATE utf8mb4_bin`。
- 在 `object_kinds()` 补 `Index`/`Trigger`（如果目录查询支持），并让 SQLite 数据范围包含 `type='view'`。
- 加实库测试：MySQL latin1 连接、SQLite 正则、SQLite 视图数据、索引/触发器结构查找。

### 3.7 监控：PostgreSQL 语义和页面状态

**位置**：`crates/navidog-sql/src/security.rs:330-394`，`crates/navidog-designer/src/tools.rs:1049-1145`

- **结束进程**：PostgreSQL 用 `SELECT pg_terminate_backend(id)`，规格 43 §5 明确要求 `pg_cancel_backend`。`pg_terminate_backend` 会直接断开整个会话，可能回滚用户事务；`pg_cancel_backend` 只取消当前查询。这是语义错误，不只是文案。
- **变量页**：PostgreSQL 的 `set_variable_sql` 生成 `ALTER SYSTEM SET name = value`，而规格说 PostgreSQL 变量只读。`ALTER SYSTEM` 需要超级用户、写入 `postgresql.auto.conf`，还要 reload 才生效；这既不是 `SET`，也不该在监控面板里提供。
- **三页没有选中态**：进程/变量/状态是三个按钮，共用 `output`；没有 `page` 状态，用户不知道当前显示的是哪一页。
- **自动刷新总是进程页**：`toggle_auto` 里固定 `this.dialect.process_sql()`，即使用户正在看变量/状态页，刷新也会切回进程。
- **移动到新窗口停止刷新**：见 3.4。

**建议**：PG 用 `pg_cancel_backend`；PG 变量页只读（隐藏/禁用 Set 按钮）；加 `MonitorPage` 状态和选中样式；自动刷新当前页；移动窗口不取消 `watch`。

### 3.8 触发器/分区表单

**位置**：`crates/navidog-designer/src/panel/more.rs:57-59`、`:163-225`

- `more_field` 的参数是 `_disabled`，但函数体完全没用它：

```rust
fn more_field(&self, key: &str, input: impl IntoElement, _disabled: bool) -> impl IntoElement {
    FormRow::new(t!(format!("designer.{key}")).to_string(), input).hug_label()
}
```

调用方也没有给 `Input` 加 `.disabled(disabled)`，所以保存/预览进行中，用户仍然可以改触发器/分区/排除表单。这会导致“预览的 SQL”和“执行时的草稿”不一致。

- 触发器表单把 `timing`/`event` 当自由文本，SQL 里硬编码 `FOR EACH ROW`：

```rust
format!("CREATE TRIGGER {quoted} {timing} {event} ON {table} FOR EACH ROW {body}")
```

规格要求时机下拉、事件多选、PostgreSQL 可关“对每一行”变成 `FOR EACH STATEMENT`。现在用户只能自己拼字符串，且无法生成 statement 级触发器。

- MySQL 分区分支在 `inline_table_options` 为 true 时直接设置 `partition` 并 return，完全忽略 `name`：

```rust
self.draft.partition = format!("PARTITION BY {method}({key}){tail}");
return Ok(());
```

规格要求每个分区有名称和边界，MySQL 的语法是 `PARTITION p0 VALUES LESS THAN (...)`。现在表单里的“名称”没有作用，生成的 SQL 缺少 `PARTITION p0`，很可能直接失败。

**建议**：`more_field` 真正禁用输入；触发器用下拉/多选，按方言生成 `FOR EACH ROW`/`FOR EACH STATEMENT`；MySQL 分区把 `name` 拼进 `PARTITION {name} …`，并校验名称。

### 3.9 PostgreSQL 分区子表没有加载

**位置**：`crates/navidog-schema/src/table_design/mod.rs:245`、`parse.rs:117`、`plan.rs:454-463`

PG 快照查询的是当前表的父表：

```sql
SELECT ... FROM pg_inherits i ... WHERE i.inhrelid = c.oid
```

没有查询 `inhparent = c.oid` 的子表；`parse.rs` 只把 `PARTITION BY ...` 放进 `design.partition`，不填 `partition_children`。因此：

- 打开已有分区表，分区页看不到现有子分区；
- `plan.rs` 的 `old.partition_children` 为空，删除子分区的 diff 永远不会触发；
- 用户只能新增子分区，不能 ATTACH/DETACH 已有子分区。

规格 41 §3 要求“已有分区表只改列表时，能 ATTACH / DETACH 的按名称做”。建议加载 `pg_inherits` 子表及 `pg_get_expr(relpartbound)`，填进 `partition_children`，并补计划测试。

### 3.10 其他逻辑边角

- **已有数据库/模式的选项不加载，dirty 也不跟踪。** `DefinitionKind::Database | Schema | Attach | Detach` 的 `catalog_kind()` 返回 `None`（`definition.rs:118`），`load()` 直接 return（`object.rs:295-296`），所以字符集/排序规则/拥有者都是空白。`dirty()` 对 `previous.is_some()` 只比较 body/name（`object.rs:202-205`），选项改动不会标脏，关闭时静默丢失。建议加载元数据，并把 options 纳入 dirty。
- **`ObjectDesigner::create` 对缺失方言直接 `expect`。** `object.rs:108-109` 在连接已删除/驱动未注册时会 panic；表设计器有 `MissingConnection` 面板兜底。建议同样返回 `MissingConnection` 或错误面板。
- **`DesignField` 绕过去重。** `main_window.rs:870-875` 直接 `TableDesigner::open` + `add_editor`，没有走 `open_editor`/`find_tab`；双击字段可能打开第二个表设计器。建议复用 `open_editor`，或在 `add_editor` 前查重。
- **`ToolPanel.dirty()` 只看 `preview`。** 用户输入了用户名/密码但还没点“预览”，关闭不会询问；移动窗口又会设置 `close_approved`。建议把表单输入纳入 dirty，或明确“只有预览后的 SQL 才算草稿”。

---

## 4. UI 设计

### 4.1 截图观察

查看的截图：

- `target/snapshots/object-designer-light.png`
- `target/snapshots/users-light.png`
- `target/snapshots/designer-options-light.png`

观察到：

1. **对象设计器的名称输入框占满整个面板宽度。** 规格 40 §8 和 41 §1 要求 300pt 表单宽度；表设计器的选项页用 `tokens::form::FIELD_WIDTH`（`panel/options.rs:147`、`panel/view.rs:186`），对象设计器的 `FormRow` 没有设置宽度，输入框被拉满。截图里“名称”一行从左边一直延伸到右边，和表设计器的表单密度不一致。
2. **定义编辑器只有 110pt 高。** `object.rs:714-717` 用 `tokens::designer::PROPERTY_MIN_HEIGHT`（110pt）作为编辑器高度；截图里编辑器下方有一大片空白。规格说定义是多行 SQL 编辑器，应该占据剩余高度，而不是固定成属性区的最小高度。
3. **SQLite 用户面板只有一行英文说明。** `users-light.png` 显示：

   > SQLite user authentication needs the user-auth extension, which this build does not include.

   中文界面里出现英文；没有标题、图标或说明卡片，下面是大片空白。规格 43 §1 要求面板说明原因，但至少应该本地化，并给出“为什么没有/如何启用”的两三行说明。
4. **选项页的复选框和“高级”按钮居中**（`designer-options-light.png`）。这个问题在 `crates/navidog-designer/src/panel/options.rs`，该文件不在本 PR 的 diff 里，属于既有问题，**不计入本次审查**。

### 4.2 表单密度与尺寸

- 新增的 `more.rs` 表单（排除/触发器/分区）直接用 `Input::new`，没有 `.w(tokens::form::FIELD_WIDTH)`，也没有 `.small()`；对象设计器的选项页用了 `.small()` 但没设宽度。结果是新页面和表设计器的字段网格/选项页看起来不像同一套 UI。
- 用户面板把整个表单限制在 `max_h(PROPERTY_MIN_HEIGHT)`（110pt，`tools.rs:802`）。MySQL 用户表单有 10 多个字段，110pt 只能显示三四行，用户必须在一个小滚动框里填完整张表。建议去掉这个上限，或让表单占满可用高度。
- `more_field` 忽略 `disabled`，保存/预览时输入框仍然可编辑；这既是逻辑问题，也是状态反馈问题（用户不知道当前不能改）。

### 4.3 反馈与空状态

- **新建视图的定义页没有提示。** `designer.yml` 里现成的 `definition_help` 是表列定义提示，而且源码里也没有使用；视图页没有任何说明文字，截图里只有一个空编辑器和行号“1”。新用户不知道这里要填 `SELECT` 还是整段 `CREATE VIEW`。
- **图标删除按钮没有 tooltip/可访问名称。** `more.rs:73-80` 的删除按钮只有 `minus` 图标，没有 `.tooltip(...)` 或文字标签；在触发器/分区列表里，用户只能靠位置猜。
- **错误只显示为一行红字。** `object.rs:683-684` 和 `tools.rs:763-764` 都是 `div().px_2().text_color(p.danger).child(error)`；长错误（例如服务器返回的多行错误）会挤在选项卡下方。表设计器也是同样模式，所以不算本 PR 独有，但新面板可以做得更好（可折叠的错误条/对话框）。
- **查找/监控的结果区没有表头、列宽、行选择。** 结果是等宽文本，长值会换行/溢出；规格要求的是可双击的结果列表。

### 4.4 文案与 i18n

- `SQLITE_AUTH_REASON` 是硬编码英文（`security.rs:10`），在中文界面直接显示；`more.rs:166-167`、`security.rs` 里大量 `DbError::msg("…")` 也是英文。`check-i18n.sh` 只检查 `t!` 字面量，不检查这些字符串，所以能通过。
- `designer.saved` 的英文是 “Table saved”、中文是“表已保存”（`designer.yml:99-101`），但对象设计器保存视图/函数/事件时也用它（`object.rs:275`）。用户保存一个视图，会收到“表已保存”。
- 触发器/分区/用户面板的字段标签有些是 `t!`，有些是英文错误；同一面板中英文混排。

### 4.5 视觉一致性

- 对象设计器用 `window_bg`，表设计器用 `control_bg`；两套 token 在当前主题里都是 `#ffffffff`/`#1e1e1eff`（`02-design-tokens.md:178-179`），所以截图里看不出差异。这不是问题，但建议统一成语义更准确的 `control_bg`，避免以后主题改色时出现不一致。
- 两者都用 `TabBar::segmented()`，页签样式本身是一致的。

---

## 5. 验证

### 5.1 本次实际运行的检查

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --all --check` | 通过 |
| `cargo clippy --workspace --all-targets --offline -- -D warnings` | 通过（仅有依赖的 future-incompat 提示） |
| `scripts/check-raw-px.sh` | 通过 |
| `scripts/check-i18n.sh` | 通过 |
| `scripts/check-dialect-branches.sh` | 通过 |
| `scripts/check-deps.sh` | 通过 |
| `cargo test -p navidog-sql -p navidog-schema -p navidog-designer --lib --offline` | 通过：12 + 33 + 21 = 66 项 |

### 5.2 Docker 实库复现

- PostgreSQL 命名约束冲突：

  ```text
  ERROR:  relation "zz_named_pkey" already exists
  ```

- PostgreSQL `serial` 默认值依赖旧序列：

  ```text
  ERROR:  cannot drop table zz_s because other objects depend on it
  DETAIL:  default value for column id of table zz_s__navidog_new depends on sequence zz_s_id_seq
  ```

- MySQL latin1 连接 + `COLLATE utf8mb4_bin`：

  ```text
  ERROR 1253 (42000): COLLATION 'utf8mb4_bin' is not valid for CHARACTER SET 'latin1'
  ```

  同一语句在 `--default-character-set=utf8mb4` 下成功。

### 5.3 没有重复跑的检查

- 没有重复跑 `cargo test --workspace --offline`；PR 验收记录声称通过。
- 没有重复跑完整的 `NAVIDOG_LIVE=1` 实库套件；本次只用 Docker 直接验证了上述三条 SQL 级问题。
- 没有逐张比对深色截图；浅色截图已看，深色同布局。

### 5.4 建议补的测试

1. **PostgreSQL 重建**：命名 PK、命名 UNIQUE、`serial`、`bigserial`、identity、带触发器的真实快照；执行后断言约束名、序列、触发器都在。
2. **对象设计器**：新建视图/函数 → 保存 → 再改 → 再保存；已有视图/函数 → 加载 → 改选项/定义 → 保存；已有数据库 → 改字符集 → 关闭时提示。
3. **移动到新窗口**：对象设计器/工具面板移动后不设置 `close_approved`，监控 `watch` 不被取消；关闭时才提示。
4. **用户**：列表选择已有用户 → 改密码/限制 → 生成 `ALTER USER`；改用户名/主机 → 生成 `RENAME USER`；密码输入遮挡。
5. **查找**：MySQL latin1 连接的大小写敏感查找；SQLite 正则；SQLite 视图数据；索引/触发器结构范围；双击打开。
6. **监控**：PostgreSQL `pg_cancel_backend`；PG 变量页只读；自动刷新当前页；移动窗口后继续刷新。
7. **UI**：排除/触发器/分区页、权限/监控/查找面板的 headless 测试和浅深色截图；表单 300pt 宽度、禁用态、空状态。

---

## 6. 结论

这个 PR 的工程骨架是好的：crate 边界、SQL 计划/执行分离、选项卡接入、i18n 和 token 使用都符合项目习惯；表设计器的计划测试也有一定覆盖。问题主要集中在“把最小实现当成完成”：

- **P0**：PostgreSQL 列重排会静默丢触发器，属于 schema 数据丢失，必须修。
- **P1**：PostgreSQL 重建在命名约束/`serial` 上失败；对象设计器已有对象 raw 化且第二次保存失败；移动到新窗口绕过关闭确认；用户面板不能编辑已有账号；查找结果不可双击且 SQLite/MySQL 有运行时错误；PG/MySQL 触发器页为空；MySQL 分区名被忽略；PostgreSQL 监控语义错误。
- **P2/P3**：PG 分区子表、信息窗格交互、函数参数网格/运行对话框、扩展双列表、数据库/模式选项 dirty、菜单能力门控、表单尺寸/禁用态、i18n 文案、`DefinitionFamily` 架构、重复格式化、测试覆盖。

**建议结论：Request changes。** 先修 P0/P1，修正 `progress.md`/验收记录的状态，再合并；如果希望缩小风险，把表设计器重建修复和对象设计器/工具拆成两个 PR。当前状态不适合作为 “M4 完成” 合并到 `main`。

---

*第 0–6 节是对 `05a1f78` 的审查结论，保留原文。第 7 节记录其后的修复进度。*

---

## 7. 修复进度

第 7.1–7.3 节是 `9dccbb6` 的修复记录。本轮只修会丢对象、生成错误 SQL，或让保存走错路径的问题。规格里还没做的交互保持原样。第 7.4 节是 `9dccbb6` 之后评论里又要修的项。

### 7.1 已修

| 审查项 | 做法 | 验证 |
| --- | --- | --- |
| **P0** PostgreSQL 列重排丢掉触发器 | `Dialect::trigger_definitions_sql`：PostgreSQL 用 `pg_get_triggerdef`，MySQL 从 `information_schema.TRIGGERS` 拼 `CREATE TRIGGER`。快照同时写入 `TableSnapshot.triggers` 和 `design.triggers`，重建在改名之后按原文发出 | 计划测试仍是手工放进 `TableSnapshot.triggers` 的文本。加载 SQL 没有实库往返 |
| **P1** 命名 PK/UNIQUE/EXCLUDE 与 `serial` 重建失败 | 索引类约束先改成临时名（后缀 `__navidog`，最长 63 字节），表改名后再 `RENAME CONSTRAINT`。`nextval` 默认值在 `DROP TABLE` 前 `DROP DEFAULT` 并 `ALTER SEQUENCE … OWNED BY NONE`，改名后写回原表达式并 `OWNED BY` 新列。CHECK 名称是表内的，不改。共享序列只被当作默认值时会改挂到新列（已知上限） | `postgres_rebuild_keeps_constraint_names_and_serial_defaults` |
| **P1** 新建对象第二次保存被外部变更守卫拒绝 | `DefinitionDraft.server_ddl` 只存服务器 DDL。守卫比较它，脏标记仍比较编辑器正文。保存成功后清空 `server_ddl`，再只回填服务器 DDL，不把正文换成 `CREATE …` | `definition_plans_cover_each_object_kind`、`raw_edits_replace_an_existing_object` 仍通过。没有「保存两次」的界面测试 |
| **P1** 移到新窗口取消任务 | `ObjectDesigner` 与 `ToolPanel` 的 `on_removed`：`tabs::removed` 为真时直接返回。真正关闭才取走任务；监控同时停掉自动刷新 | 没有 headless 测试 |
| **P1** 密码未遮挡 | 密码框 `InputState.masked(true)`，并打开 `mask_toggle`。取值仍是明文，账户 SQL 不受影响 | 没有界面测试 |
| **P1** SQLite 正则、MySQL latin1 大小写 | SQLite 在生成 SQL 前返回 “This SQLite build has no REGEXP function.”。MySQL 区分大小写的 LIKE 用 `LIKE BINARY`，正则用 `CAST(列 AS BINARY) REGEXP` | `security_statements_cover_each_product` |
| **P1** PostgreSQL/MySQL 触发器页为空 | 与 P0 同一条快照加载 | 同上，没有实库往返 |
| **P1** MySQL 分区名被忽略 | 填了名称时生成 `PARTITION BY … (PARTITION 名称 范围)`，空名称仍用原来的 `PARTITIONS n` 尾部 | `mysql_partition_keeps_the_partition_name` |
| **P1** 监控语义 | 结束进程改为 `pg_cancel_backend`。PostgreSQL `set_variable_sql` 返回只读错误，变量名/值和设置按钮只在 MySQL 显示。进程/变量/状态记住当前页并标选中，自动刷新走当前页 | `security_statements_cover_each_product` 覆盖结束进程、只读变量和 `LIKE BINARY`。页选中态没有界面测试 |
| **P2** `more.rs` 保存时输入框仍可编辑 | 排除/触发器/分区页的输入在禁用时传 `.disabled(disabled)` | 没有界面测试 |
| 排除元素里的换行被 `flush` 拆成多条约束 | `EXCLUDE` 的元素和 `WHERE` 写入约束编辑器前收成一行 | `forms_round_trip_and_unknown_clauses_stay_advanced` |

已有对象打开时仍把正文设成整段服务器 DDL（`raw = true`）。二次保存不再把编辑器里的 `SELECT` 拿去和 `CREATE VIEW` 比较；结构化加载（选项页、参数网格）没有做。

### 7.2 未修

这些留在规格和审查里，本轮不实现：

- 已有视图、函数、过程、事件、索引、触发器仍按整段 DDL 打开，选项页隐藏。
- 用户面板不能选择或修改已有账号，保存已有用户仍走 `CREATE USER`。密码遮挡已修。
- 查找结果仍是字符串，不能双击；没有连接/库/模式选择器；对象种类不含索引和触发器；SQLite 数据搜索跳过视图。
- PostgreSQL 已有分区子表不从 `pg_inherits` 加载，不能按名称拆下。
- 触发器表单仍是自由文本，事件固定 `FOR EACH ROW`。
- 已有数据库/模式的选项不加载，选项改动不标脏。
- `ObjectDesigner::create` 在方言缺失时仍 `expect`。
- `DesignField` 不走 `open_editor` 去重。
- `ToolPanel.dirty()` 只看已生成的预览。
- 信息窗格「使用」不可点击；成员页仍隐藏；MySQL「被使用」只读外键。
- 表单宽度、定义编辑器高度、`SQLITE_AUTH_REASON` 英文、`designer.saved` 对视图也显示「表已保存」、`DefinitionFamily` 产品分支、菜单用无关能力标志推断产品、`ToolPanel` 的体积。
- `progress.md` 里 F-DSG-005/006/008、F-DSG-011 至 F-DSG-019、F-INFO-003/008、F-SEC-001 至 F-SEC-004、F-TOOL-001/002 仍标「完成」。缺口写在 `progress.md` 第 4 节，本轮不把这些行整批改成「部分」。F-DSG-010 的列重排数据丢失和重建失败已修，该行保持「完成」，验证列补了新测试名。

### 7.3 本轮跑过的检查

- `cargo fmt --all`
- `cargo clippy -p navidog-sql -p navidog-schema -p navidog-designer -p navidog-mysql -p navidog-postgres --all-targets -- -D warnings`
- `cargo test -p navidog-schema -p navidog-designer --lib -- postgres_rebuild_keeps postgres_column_reorder security_statements mysql_partition definition_plans raw_edits`（6 项通过）

没有跑 Docker 实库，也没有跑 `cargo test --workspace`。审查第 5.2 节的三条 SQL 错误没有在服务器上再复现一遍。

### 7.4 `9dccbb6` 之后的评论

| 评论 | 处理 |
| --- | --- |
| 保存后的回读失败会清空 `server_ddl`，下一次保存跳过外部变更检查 | 已有对象的基线为空时拒绝保存，并提示关闭选项卡后重新打开。回读失败或读到空定义时同样提示 |
| `object_ddl` 失败被当成对象不存在，保存继续执行 | 读失败直接返回错误，不执行计划。`a_failed_conflict_read_does_not_run_the_save` |
| 新建列的 `nextval` 会把别的表的序列改挂过来 | 只对原来就有的列做 `OWNED BY`。已有列指向共享序列时，计划里仍然没有归属信息，仍会改挂（已知上限）。`postgres_rebuild_keeps_constraint_names_and_serial_defaults` 断言新列的 `other_seq` 不会被 `ALTER SEQUENCE` |
| MySQL 区分大小写的 `CAST(列 AS BINARY) REGEXP` 在 8.0.22 起报错 | 改为在模式前加 `(?-i)`，列保持原来的字符集。MariaDB 的 PCRE 也认这个标志。`security_statements_cover_each_product`；实库 `live_mysql_case_sensitive_regexp_matches_without_binary` |
| 第 7 节仍写着未提交 | 本节已改为 `9dccbb6`，补充记在 7.4 |
| 触发器要有 PostgreSQL 实库往返，包括改过的定义 | `live_postgres_reorder_keeps_a_loaded_and_edited_trigger`：从目录读出触发器，改定义并重排列后，插入能走到新触发器。这条和上面的 MySQL 正则实库测试已在本机通过 |

命名约束那条评论已由对方标为解决，排除约束换行也已解决，不再改。

### 7.5 `33a7255` 之后没贴进行内的评论

| 评论 | 处理 |
| --- | --- |
| 用户预览把密码明文写在 `IDENTIFIED BY` / `PASSWORD` 里 | 显示文本把这两处字面量换成 `'***'`。真正执行的 `preview` 仍是原语句。`account_preview_hides_the_password_and_leaves_the_same_text_elsewhere` |
| SQLite 触发器正文已以 `;` 结尾时，拼出来是 `;; END` | 拼 `BEGIN … END` 前去掉正文末尾的分号，语句中间的分号保留。`sqlite_trigger_body_keeps_one_terminator` |
| `COMMIT` 失败被忽略，界面仍当成已保存 | `COMMIT` 失败时回滚并返回该错误。`a_failed_commit_is_not_reported_as_saved` |

更早的行外评论里，关闭对话框和空查找标题已在 `05a1f78` 处理。已有账号编辑、已有对象结构化编辑和按产品生成 DROP 仍不做，与第 7.2 节一致。

### 7.6 `44668cf` 之后的评论

| 评论 | 处理 |
| --- | --- |
| 正文以 `--` 注释结尾时，`; END` 被注释吃掉 | 用 SQLite 词法切出末尾的行注释，分号放在注释前，`END` 单独占下一行。`sqlite_trigger_body_keeps_one_terminator` |
| 第 7.5 节把已有账号编辑写成已在 `05a1f78` 处理，和第 7.2 节矛盾 | 已从已处理列表去掉。已有账号编辑仍未做 |
| 只改列名时如果同时改了触发器，快路径会丢掉触发器修改 | 快路径要求解析后的触发器列表不变，否则走整表重建。`sqlite_rename_with_a_trigger_edit_rebuilds` |
| 信息窗格的成员页要从导航器选中用户或角色 | 未做。导航器还没有用户和角色对象，与第 7.2 节一致 |

### 7.7 `272f73b` 之后的评论

| 评论 | 处理 |
| --- | --- |
| 列重命名同时改了别的触发器时，重建会用旧列名重建没改过的触发器，`UPDATE OF` 不再触发，视图也留着旧列名 | 这种编辑不再整表重建。先替换文本变了的触发器，再 `RENAME COLUMN`，让 SQLite 改写没改过的触发器、新触发器正文、视图和外键。`sqlite_rename_with_a_trigger_edit_keeps_the_other_trigger`、`sqlite_rename_and_trigger_edit_keeps_update_of_and_the_view`。第 7.6 节里的 `sqlite_rename_with_a_trigger_edit_rebuilds` 已改成前一个测试 |
| 这条重建会留下带外键的备份表，删光子行后仍删不掉父行 | 同一条路径不再产生备份表。加列这类本来就要重建的保存仍会留下备份表，这是原有行为 |
| 对象已被外部删除时，空的 `object_ddl` 会跳过冲突检查，保存把对象建回来 | 当前定义和打开时读到的定义不一致就拒绝，空字符串也算。`a_deleted_object_is_not_recreated` |

### 7.8 `8ca59f7` 之后的评论

| 评论 | 处理 |
| --- | --- |
| 用户保存成功后基线不更新，页签仍标脏，下次再发 `CREATE USER` 或再 `RENAME` | 成功执行后把当前草稿记成已保存：`previous_name` 换成现名，权限基线换成当前勾选，并清空密码框。`a_saved_account_is_altered_instead_of_created_again` |
| 打开已有 PostgreSQL 角色时不读属性，只改密码也会写成默认的 `LOGIN` / `NOSUPERUSER` | 从 `pg_roles` 读 `rolcanlogin`、`rolsuper`、`rolcreatedb`、`rolcreaterole`、`rolinherit`、`rolreplication`、`rolconnlimit`、`rolvaliduntil`，填进表单后再记基线。`role_profile_sql`、`a_loaded_postgres_role_keeps_nologin_and_superuser` |
| 已有账号只改认证插件、密码为空时发出 `IDENTIFIED WITH plugin`，会换掉现有口令哈希 | 这种情况预览直接报错，要求填写密码。新建账号仍可以只指定插件。`security_statements_cover_each_product` |
| 预览评论的隐藏标记不限作者，字段原样拼进 `gh run download` | 只认 `github-actions[bot]` 的评论。URL、运行号、提交、产物名和 SHA-256 对不上字符集就丢掉这条记录。`preview-comment.sh --check` |

对象设计器保存后用编辑器正文当冲突基线的那条，代码已经改成比较 `server_ddl`（`9dccbb6`、`33a7255`），本轮不再改。`PARTITION BY` 子串切分仍按上一轮推迟。页签键盘导航对方已接受。

第 7.6 节写的「已有账号编辑仍未做」已被 `8ca59f7` 的用户列表和设计器取代。实库创建账号本轮仍未跑。

### 7.9 `7b46532` 之后的评论

| 评论 | 处理 |
| --- | --- |
| 预览生成后又改了表单，执行的仍是旧 SQL，成功后却把当前表单记成已保存，下一次预览会漏掉没执行的修改 | 生成预览时记下那份草稿、权限和密码。成功后只采用这份快照。之后改过的 SSL 或权限仍标脏，密码只有和预览里的一致才清空。`a_later_edit_stays_dirty_after_the_previewed_account_is_saved`、`security_statements_cover_each_product` |

`PARTITION BY` 子串切分仍按上一轮推迟。

### 7.10 `1ea02cd` 之后的评论

| 评论 | 处理 |
| --- | --- |
| 执行过程中再按保存会换掉 `account_preview`，查询跑的仍是旧 SQL，成功后却把后来的设置记成已保存 | 开始执行时复制一份快照。成功只采用这份，查询进行中不再生成新预览。失败则丢掉这份进行中的快照。`a_running_save_keeps_the_snapshot_from_when_it_started` |
