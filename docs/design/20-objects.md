# M2 规格：对象树、对象选项卡与信息窗格

覆盖功能：
- **导航与对象**：F-NAV-001（到对象层级）、F-NAV-002 ~ F-NAV-007、F-OBJ-001 ~ F-OBJ-002、F-OBJ-004 ~ F-OBJ-008；
- **信息窗格**：F-INFO-001、F-INFO-002、F-INFO-007、F-INFO-010；
- **应用与连接**：F-APP-005（编辑器选项卡与弹出窗口）、F-APP-008（专注模式）、F-APP-011 ~ F-APP-015、F-CONN-015、F-CONN-016、F-CONN-019、F-CONN-021；
- **偏好设置**：F-PREF-003、F-PREF-005。

行为来源：`navigation_pane.html`、`object_pane.html`、`information_pane.html`、`virtual_group.html`、`favorites.html`、`focus.html`、`share_uri.html`、`con_manage.html`、`mysql_table.html`、`postgresql_table.html`、`sqlite_table.html`。

本里程碑新建 crate `navidog-navigator`，把 M1 暂放在 `navidog-connection` 中的导航窗格、对象选项卡、信息窗格迁移过去。`navidog-connection` 只保留连接对话框、会话和连接相关命令。

## 1. 技术验证：导航树（M2 第一周完成，结论写回本节）

M1 用 `gpui-component` 的 `Tree` 实现了单选、键盘导航和右键原生菜单。M2 新增的需求：

| 需求 | 现状 | 验证方法 |
| --- | --- | --- |
| 多选（⌘ 点按、⇧ 点按） | `TreeState` 只有单选 | 在 `render_row` 外层维护选择集合，自行绘制选中背景；`TreeState` 的选中项只作为键盘焦点 |
| 拖放（连接拖进组、对象拖进虚拟组） | 不支持 | 行元素加上 GPUI 的 `on_drag` / `on_drop`，验证与 `uniform_list` 的滚动能否共存 |
| 原地重命名 | 不支持 | 被重命名的行用 `Input` 替换标签 |
| 大量对象（单库 5000 张表） | 虚拟化，行高固定 | 生成 5000 张表，测量滚动帧时间（`gpui-fps` 或 `profiler` feature） |

判定规则：四项都能在外层实现，并且 5000 行滚动时单帧不超过 8.3ms，就继续用 `Tree`；任何一项需要改动 `Tree` 内部，就基于 `gpui-base` 的 `VirtualList` 自建 `NavTree`。

**结论：继续用 `Tree`。** 四项都在外层实现，没有改动 `Tree` 内部：

- **多选**：选择集合放在 `Workspace`（主选择加附加选择），`TreeState` 的选中项只作为主选择和键盘焦点；⌘ 点按、⇧ 点按的规则在 `navidog-navigator/src/selection.rs`，有单元测试。
- **拖放**：行（`ListItem`）上加 `on_drag`、`drag_over`、`on_drop`，和 `uniform_list` 的滚动可以共存。拖动一个已选中的连接时，所有选中的连接一起移动。
- **原地重命名**：被重命名的行把标签换成 `Input`。注意行是由 `TreeState` 自己的视图渲染的，只改导航面板的状态不会重绘行，必须同时通知 `TreeState`。
- **5000 个对象**：验证程序是 `cargo run --release -p navidog-navigator --example tree_spike`（280 × 900 的 headless Metal 窗口；一个分类下 5000 个对象，每行都挂着多选和拖放的处理器，其中一行是重命名用的 `Input`；各测 300 帧）：

| 场景 | 平均 | p95 | 最大 | 超过 8.3ms 的帧 |
| --- | --- | --- | --- | --- |
| 逐行滚动（每帧 3 行） | 1.4ms | 2.7ms | 3.4ms | 0 / 300 |
| 跳跃滚动（每帧 397 行） | 2.6ms | 3.1ms | 3.5ms | 0 / 300 |

debug 构建平均为 2.0ms 和 4.8ms。多选、拖进组、拖到空白处回到顶层、新建组和重命名组由 `crates/navidog-navigator/tests/navigator.rs` 通过真实的点击、拖动和键盘输入验证。

⇧ + 上/下方向键按可见行扩展选择（规则与 ⇧ 点按相同，锚点是上次普通选择）。方向键由导航面板处理，因为点选行之后焦点在面板上，树控件自己的方向键绑定收不到。拖动进行中、指针在树的可视区域内且距上沿或下沿不足一行时，每 80ms 向该方向滚动一行；指针停在边缘也会继续滚（拖动本身每帧重绘）。指针在树以外（例如右侧的对象列表）不滚动。拖动组和对象随 F-NAV-007 已实现。

## 2. 树结构

```
连接（MySQL）
└── 数据库
    ├── 表
    │   └── actor
    │       ├── 字段（偏好设置开启“显示表下的对象”时）
    │       ├── 索引
    │       ├── 外键
    │       └── 触发器
    ├── 视图
    ├── 函数
    ├── 事件 [My]
    ├── 查询（M3）
    ├── 备份（M9）
    └── 模型（M7）
连接（PostgreSQL）
└── 数据库
    └── 模式
        ├── 表、视图、实体化视图、函数、序列、类型、查询、备份……
```

- **对象分类**：分类节点的展开由导航树控制；分类下的对象在首次展开时按需加载，并缓存到连接会话的 schema 缓存里。⌘R 刷新当前节点及其子树。
- **查询后的更新**：同一连接的查询或命令列执行完成后，重新请求已加载的数据库列表、模式、对象、字段、表下元素和 DDL 缓存；已打开的视图等 DDL 选项卡随缓存结果更新。只重新加载已有缓存项，不主动展开整棵树；不同连接互不影响。新的请求替代旧请求，过期结果不得覆盖新数据。
- **显示模式下的对象**（F-PREF-002）：默认开启，导航树在数据库（没有模式的方言）或模式下显示对象分类。关闭后树停在数据库或模式，对象只在对象选项卡里看。切换后立刻生效。
- **显示表下的对象**（F-PREF-002）：默认开启。关掉后表下面不列出字段、索引、外键和触发器。
- **分类集合**：每种方言的分类集合由驱动通过 `Relational::object_kinds()` 声明，界面只负责渲染。
- **对象节点**：双击表，在对象窗格中打开表查看器（M2 第二部分，见 [21-table-viewer.md](21-table-viewer.md)）；双击其他对象，打开对应的设计器（M4），M2 中先打开只读的 DDL 选项卡。
- **展开**：单击只选中，不展开。双击和 Return：关闭的连接会打开，对象会打开，其余节点展开或折叠。行首箭头同样展开或折叠（关闭的连接会打开），但表只展开其下的字段、索引、外键和触发器，不打开表查看器。方向键右展开（关闭的连接会打开），左折叠。
- **字段图标**：表下的字段里，主键用钥匙图标（`key`，黄色），其它字段仍用列图标（`columns-3`）。是否主键读目录里的 `ColumnInfo.primary_key`，不写进节点 id。

## 3. 驱动接口扩展（`navidog-db`）

```rust
pub enum ObjectKind { Table, View, MaterializedView, Function, Procedure, Event, Trigger, Index, Sequence, Type, Schema }

pub struct ObjectInfo {
    pub name: String,
    pub kind: ObjectKind,
    pub comment: Option<String>,
    /// 详细信息视图的列：行数、数据长度、引擎、创建时间、修改时间、排序规则等
    pub properties: Vec<(String, String)>,
}

pub trait Relational: Send {
    // M1 已有：list_databases、execute
    fn list_schemas<'a>(&'a mut self, database: &'a str) -> BoxFuture<'a, DbResult<Vec<String>>>;
    fn object_kinds(&self) -> &'static [ObjectKind];
    fn list_objects<'a>(&'a mut self, scope: &'a Scope, kind: ObjectKind) -> BoxFuture<'a, DbResult<Vec<ObjectInfo>>>;
    fn object_ddl<'a>(&'a mut self, scope: &'a Scope, kind: ObjectKind, name: &'a str) -> BoxFuture<'a, DbResult<String>>;
    fn table_columns<'a>(&'a mut self, scope: &'a Scope, table: &'a str) -> BoxFuture<'a, DbResult<Vec<ColumnInfo>>>;
}

pub struct Scope { pub database: String, pub schema: Option<String> }
```

元数据查询：

| 内容 | MySQL / MariaDB | PostgreSQL | SQLite |
| --- | --- | --- | --- |
| 表 | `information_schema.TABLES`（`TABLE_TYPE='BASE TABLE'`），带 `TABLE_ROWS`、`DATA_LENGTH`、`ENGINE`、`CREATE_TIME`、`TABLE_COMMENT` | `pg_class` 加 `pg_namespace`（`relkind in ('r','p')`）、`obj_description`、`pg_total_relation_size` | `sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'` |
| 视图 | `information_schema.VIEWS` | `pg_views` | `sqlite_schema WHERE type='view'` |
| 实体化视图 | 无 | `pg_matviews` | 无 |
| 函数和过程 | `information_schema.ROUTINES` | `pg_proc`（`prokind`） | 无 |
| 事件 | `information_schema.EVENTS` | 无 | 无 |
| 字段 | `information_schema.COLUMNS` | `pg_attribute` 加 `format_type` | `PRAGMA table_xinfo` |
| DDL | `SHOW CREATE TABLE` / `VIEW` / `FUNCTION` | 由目录表拼出 `CREATE TABLE`；视图用 `pg_get_viewdef`，函数用 `pg_get_functiondef` | `sqlite_schema.sql` |

## 4. 对象选项卡

- **对象工具栏**：高度为 `object_toolbar.height`。按钮随分类变化，以表为例：打开表、设计表、新建表、删除表、导入向导、导出向导。没实现的按钮显示为禁用。
- **三种视图**：列表（默认）、详细信息、ER 图表（M7）。通过对象工具栏右侧的分段按钮切换，选择会按连接分别记住。
- **详细信息视图**：用 `DataTable`，表头高 `list.header.height`，可按列排序；“查看 > 选择列”打开列选择弹窗，选择按对象类型分别保存。
- **表概况**（F-OBJ-007）：“查看 > 显示表概况”。勾选后，对象列表底部显示当前选中对象已加载的属性（行数、数据长度、引擎等），键名与详细信息视图的列名相同，不另查一次目录。没有属性的选择不显示这条。
- **其他**：空白处的右键菜单，与导航窗格里分类节点的右键菜单相同；状态栏显示“N 个项目”或“N 个项目（已选择 M 个）”。

## 5. 编辑器选项卡

- 对象窗格的中央 `TabPanel` 中，“对象”选项卡固定在第一个：拖到别的位置会回到最左。选项卡条横向滚动时它仍会跟着走（gpui-kit 的 `TabBar` 把整条一起滚动，不能把单个选项卡留在滚动区域外）。编辑器选项卡（表查看器、DDL、以及之后的查询编辑器和设计器）依次加在后面，标题前显示对象图标；设置了连接颜色时，选项卡底部显示该颜色的细条。
- 右键选项卡：关闭、关闭其他、关闭右侧、弹出为新窗口、加到收藏夹、加到启动画面。
- 弹出为新窗口：用 `gpui_kit::open_window` 打开一个只包含该编辑器的窗口；关闭窗口时，询问是否放回主窗口。
- 偏好设置的“选项卡”页：选择在新选项卡还是新窗口中打开，以及启动画面列表（F-PREF-003）。

## 6. 信息窗格

- 标题栏左侧是一组按钮，按选择的对象显示：常规、DDL、使用、被使用、字段等。M2 实现常规、DDL、字段三个页。
- **DDL 页**：只读的 `Editor`，开启 `tree-sitter-sql`，⌘F 搜索；DDL 内容按对象缓存，⌘R 刷新。
- 切换到表查看器或 DDL 选项卡时，把工作区选择设为该选项卡的对象，信息窗格和导航树跟着选项卡走。对象选项卡激活时不改选择。
- 在导航栏单击或用方向键改选择时，如果新选择不是当前编辑器选项卡的那张表（表下的字段仍算这张表），就激活“对象”选项卡，信息窗格跟着导航栏。点的就是当前这张表则留在表查看器。

## 7. 其他功能

- **虚拟组**（F-NAV-007）：每个连接一份，保存在该连接目录的 `.vgroups.json`（作用域不重复连接 ID）；右键“管理组”可以新建组、添加到组、从组中排除；支持拖放。
- **收藏夹**（F-APP-011）：保存在 `favorites.json`；新增“收藏夹”菜单；⌥⇧1-9 添加，⌃⇧1-9 打开。
- **navidog URI**（F-APP-014、F-APP-015）：格式为 `navidog://open?conn=<名称>&host=<主机>&db=<库>&schema=<模式>&kind=<分类>&name=<对象>`；连接映射规则见 `share_uri.html`。
- **管理连接窗口**（F-CONN-015）：用表格列出全部连接；工具栏提供新建、编辑、标星、隐藏、删除；搜索时匹配名称、主机、端口和用户名，并高亮匹配内容。
- **重载**（F-CONN-019，仅 MySQL）：在对话框里勾选要执行的 FLUSH 选项后执行。

## 8. 验收清单

- [x] 查询执行后，同连接已加载的目录、字段、索引和打开的 DDL 更新，其他连接及未加载节点保持隔离，过期请求不覆盖新值。（`database_changes_reload_cached_metadata_and_open_ddl_without_touching_other_connections`、`a_replaced_metadata_load_cannot_restore_older_values`；见 [验收记录](../acceptance-query-refresh.md)。）
- [x] 技术验证结论写回第 1 节，并按判定规则选定导航树的实现方式。
- [x] 三种数据库都能逐级展开到表、视图、函数（PostgreSQL 还有模式），字段、索引、外键在偏好设置开启后显示。
- [ ] 对象选项卡的列表视图和详细信息视图与 Navicat 并排目测一致；详细信息视图可排序、可选择显示的列。
- [x] 双击表打开表查看器选项卡；选项卡可以关闭，也可以弹出为新窗口。
- [x] 信息窗格的 DDL 页显示正确的 DDL，并且有语法高亮。
- [x] 多选和拖放：⌘ 点按多选，把连接拖进组。
- [x] 一个库有 5000 张表时，展开和滚动都流畅。
