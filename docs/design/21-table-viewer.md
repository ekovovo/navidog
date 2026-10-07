# M2 规格：表查看器与结果网格

覆盖功能：F-DATA-001 ~ F-DATA-014、F-OBJ-008（打开表）、F-INFO-007。结果网格将来会被查询结果（M3）复用，所以放在单独的 crate `navidog-grid` 中。

行为来源：`data_viewer.html`、`data_rdbms.html`、`data_edit.html`、`data_filter.html`、`data_filter_wizard.html`、`data_sort_find.html`、`data_format.html`、`data_navigation_bar.html`、`data_raw.html`、`data_assistant_editors.html`、`data_field_comment.html`。

尺寸来源：`TableViewViewController`、`GridViewController`、`FilterAndSortManager`、`FormViewController`。

## 1. 技术验证：结果网格（M2 开工后先做，结论写回本节）

`gpui-component` 的 `DataTable` 已经提供：
- 行、列两个方向的虚拟化；
- 左侧固定列；
- 列宽调整、列移动；
- 由 delegate 实现的排序和右键菜单；
- 无限加载（`has_more`、`load_more`）。

它缺少 Navicat 网格的几项核心能力：

| 需求 | 在外层实现的方式 |
| --- | --- |
| 矩形区域选择，⇧ 和 ⌘ 扩展选择 | 选择模型由 `GridState` 自己维护；`render_td` 根据它绘制选中背景；单元格上的 `on_mouse_down`、`on_mouse_move` 实现拖拽选择；`DataTable` 自带的单格选择只用作键盘焦点 |
| ⇧ + 方向键扩展选择 | 网格外层加 `key_context("ResultGrid")` 和 Action：方向键在焦点移动后同步选择，⇧ + 方向键修改选区 |
| 就地编辑 | 双击或回车时，在 `render_td` 中把正在编辑的单元格渲染成 `Input`；Esc 取消；Tab 和回车提交并移到下一格 |
| 修改标记 | 修改过的单元格、新增行、待删除行分别用不同背景表示（颜色按 Navicat 实测，登记进 token 表） |
| 行号列 | 第 0 列作为固定列，宽 `grid.row_number_width` |

判定规则：在 10 万行、20 列的数据上，矩形选择、键盘扩选、就地编辑都能工作，并且滚动时单帧不超过 8.3ms，就采用 `DataTable` 加外层补充；否则基于 `gpui-base` 的 `Table` 原语和 `VirtualList` 自建 `ResultGrid`。不管选哪种，对外接口都一样（第 2 节），选择不影响上层代码。

**结论：采用 `DataTable` 加外层补充。** 验证程序是 `cargo run --release -p navidog-grid --example spike`（1440 × 900 的 headless Metal 窗口，10 万行 × 20 列，每次滚动后测一帧的布局与绘制时间，各 300 帧）：

| 场景 | 平均 | p95 | 最大 | 超过 8.3ms 的帧 |
| --- | --- | --- | --- | --- |
| 逐行滚动（每帧 3 行） | 6.1ms | 6.9ms | 29.6ms | 5 / 300 |
| 跳跃滚动（每帧 997 行） | 6.1ms | 7.0ms | 11.2ms | 2 / 300 |

- 偶发的超时帧来自首次排版没见过的文本（每行内容都不同），自建网格同样要经过这一步，所以不影响结论。debug 构建平均约 10.8ms。
- 矩形选择、⇧ + 方向键扩选、就地编辑、复制都由 `crates/navidog-grid/tests/grid.rs` 通过真实的点击和按键验证。
- 实现中与上表设想不同的两点：
  - `DataTable` 只在按下和抬起是同一格时发出 `SelectCell`，所以选区在单元格的 `on_mouse_down` 中更新，拖拽通过 `on_mouse_move` 扩展；
  - 行号列是 delegate 的第 0 列（固定在左侧），显示当前行、已修改、新增和待删除的标记，不显示行号，与 Navicat 一致。
- 已知差异：每个可排序列的表头都会显示 `DataTable` 的默认排序图标，而 Navicat 只在已排序的列上显示。

## 2. `navidog-grid` 对外接口

```rust
pub struct GridData {
    pub columns: Vec<GridColumn>,      // 名称、类型、是否主键、是否可为空、注释
    pub rows: Vec<Vec<CellValue>>,     // 当前页
    pub total_rows: Option<u64>,       // COUNT(*)，异步获得
}

pub enum CellValue { Null, Text(SharedString), Binary(usize /* 字节数 */) }

pub struct GridState {
    // 数据、选择、编辑中的单元格、修改集合、列宽、隐藏列、行高
}

pub enum GridEvent {
    SelectionChanged,
    CommitRequested(ChangeSet),   // 离开已修改的行、点击“应用”、⌘S
    LoadPage(u64),
}

pub struct ChangeSet {
    pub updates: Vec<RowUpdate>,  // 定位行的主键值（没有主键时用整行旧值）加上修改过的列
    pub inserts: Vec<Vec<CellValue>>,
    pub deletes: Vec<RowKey>,
}
```

`ChangeSet` 转成 SQL 的工作由 `navidog-db` 中各方言的 `Dialect::render_changes` 完成，并配有单元测试覆盖：标识符引用、NULL 值、二进制值、没有主键的表。

## 3. 表查看器布局

```
┌─────────────────────────────────────────────────────────────┐
│ 工具栏：开始事务 | 文本 | 筛选 | 排序 | 导入 | 导出 | 生成数据 │
├─────────────────────────────────────────────────────────────┤
│ 筛选与排序窗格（⌘⇧F 显示或隐藏）                              │
├────┬────────────────────────────────────────────────────────┤
│ 行号│ 网格（表头高 28）                                       │
│ 20 │                                                         │
├────┴────────────────────────────────────────────────────────┤
│ 底部栏（高 28）：+ − ✓ ✗ ⟳ 停止 | SQL 预览 | ⏮ ◀ [页] ▶ ⏭ | 限制 [1000] 条记录每页 │
└─────────────────────────────────────────────────────────────┘
```

- **网格视图和表单视图**：网格视图之外还有表单视图（`FormViewController`），一次编辑一条记录，可以随时切换。
- **横向滚动**：查看器把网格限制在选项卡宽度内（`min_w_0`），列超出时由 `DataTable` 的横向滚动条滚动，滚动条保持可见、可以拖动。触控板横向滑动同样滚动。不要把选项卡撑宽后被停靠区裁掉。
- **单元格编辑器**：显示在网格下方的可调整面板里，也可以选择显示在信息窗格中。按类型提供文本、十六进制、图像、JSON、枚举和集合编辑器。

## 4. 数据加载

- 打开表时执行 `SELECT * FROM <表> [WHERE …] [ORDER BY …] LIMIT <每页> OFFSET <偏移>`，同时另起一个查询执行 `COUNT(*)`，显示在底部栏里。
- 每页的记录数默认是 1000（偏好设置的“记录”页，F-PREF-005），可在底部栏修改，也可以取消“限制”，一次加载全部。
- 加载过程中，“停止”按钮调用会话的 `cancel_handle`。
- 表查看器为自己开一个独立的会话，不和导航树的元数据会话共用，避免长时间查询阻塞元数据浏览。

## 5. 编辑与事务

- **提交时机**：离开已修改的行、点击 ✓ 或按 ⌘S 时提交；点击 ✗ 放弃修改。
- **未开启事务时**：每次提交独立执行。提交失败时弹出数据库的错误信息，修改保持未提交状态。
- **开启事务后**：工具栏显示“提交”和“回滚”；关闭选项卡前如果事务还没结束，询问是提交还是回滚。偏好设置里可以设为自动开始事务。
- **无主键的表**：用整行的旧值定位，MySQL 加上 `LIMIT 1`；同时在状态栏提示“该表没有主键，可能影响多行”。
- **原始数据模式**（F-DATA-010）：“查看”菜单和表查看器工具栏切换。开启后，字符串列的值在网格里显示为带引号的 SQL 字面量（`PENELOPE` 显示为 `'PENELOPE'`，引号按方言转义）；数字等其它类型不加引号。之后输入的非空文本按 SQL 表达式写入（如 `NOW()`、`UPPER('a')`）。用来定位行的旧值仍是字面量。关闭后恢复不带引号的显示，一切按字符串字面量处理。已经输入的表达式保持当时的写法。

## 6. 筛选与排序

- **可视化条件**：条件以树形结构组织，每条包含字段、运算符和值，可以加括号分组、取反。运算符照 `data_filter_wizard.html` 的完整列表实现。
- **“文本”模式**：直接编辑 WHERE 子句；从可视化条件切换到文本时自动生成对应的文本，反向切换时如果解析失败，就保留文本模式。
- **右键菜单快速筛选**：字段 = 值、字段 != 值、字段 like 值等，直接加入筛选条件。
- **排序**：可以多字段排序；点击表头切换升序、降序、不排序；排序条件生成 ORDER BY，交给服务器执行。

## 7. 复制与粘贴

- **复制**：⌘C 把选区复制为制表符分隔的文本。“复制为”子菜单提供 INSERT 语句、UPDATE 语句、TSV（仅数据、仅字段名、字段名和数据）。
- **粘贴**：多行多列的文本按当前选区的左上角对齐写入，超出已有行数时追加新行。

## 8. 新增 token（登记到 02-design-tokens.md）

| token | 值 | 来源 |
| --- | --- | --- |
| `grid.header.height` | 28 | nib:GridViewController/NSTableHeaderView（高 28） |
| `grid.row_number_width` | 20 | nib:GridViewController/行号 NSScrollView（宽 20） |
| `grid.row.height` | 待 AX | 暂定 20 |
| `viewer.bottom_bar.height` | 28 | nib:TableViewViewController/底部栏（1153 × 28） |
| `viewer.bottom_bar.button` | 20 × 16 | nib:TableViewViewController/NSButton |
| `viewer.limit.width` | 212 | nib:TableViewViewController/“Limit … records per page”（212 × 24，11pt） |
| `viewer.pager.width` | 86 | nib:TableViewViewController/分页按钮组（86 × 27，按钮间距 28） |
| `filter.segmented.width` | 115 | nib:FilterAndSortManager/NSSegmentedControl（115 × 24） |

## 9. 验收清单

- [x] 技术验证结论写回第 1 节。
- [ ] 打开 sakila.actor、pagila.public.actor、SQLite 测试表，分页、总行数、筛选、排序都正确。
- [ ] 编辑、新增、删除后提交，数据库中的数据正确；提交失败时显示原始错误，修改保留。
- [ ] 开启事务后，提交和回滚都正确；关闭选项卡时提示未结束的事务。
- [ ] 区域选择后复制粘贴，以及“复制为 INSERT/UPDATE”的输出格式正确。
- [ ] 10 万行数据滚动流畅。
