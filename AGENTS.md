# navidog Agent 指南

navidog 用 gpui-kit 0.7 复刻 Navicat Premium 17（macOS）。决策、架构和工程规则的唯一来源是 [docs/design/00-overview.md](docs/design/00-overview.md)，术语以 [CONTEXT.md](CONTEXT.md) 为准；本文只补充开发流程和代码里看不出来的坑。

## 开工

1. 读 [docs/progress.md](docs/progress.md)，确定当前里程碑和要做的任务。
2. 读该任务对应的模块规格（`docs/design/1x-*.md`、`2x-*.md`……）和它引用的功能编号（`F-…`，见 `01-feature-inventory.md`）。规格缺少尺寸、默认值或行为时，先补规格；规格和代码分开提交。
3. Navicat 的尺寸、文案、行为从本地 `reference/` 查（导出命令见 00-overview 第 2 节）。`reference/` 只在本地使用，内容不进仓库；Navicat 的图标和 logo 不提取。

## 写代码

- **依赖方向**：feature crate（`navidog-connection`、`navidog-navigator`、`navidog-grid`……）之间不互相依赖，唯一的例外是 `navidog-query` 依赖 `navidog-grid` 复用结果网格。跨 feature 的命令放 `navidog_core::actions`，会话放 `navidog_db::sessions`，由外壳 `crates/navidog` 组合面板（按对象类型选编辑器在 `src/editors.rs`）。
- **纯函数**：不依赖界面、也不需要驱动的工具函数放 `navidog-util`。SQL 类型名归类、enum/set 列表、十六进制和 JSON 文本在 `navidog_util::sql_type` 和 `navidog_util::format`。图标和颜色留在画它们的 crate。需要驱动或方言的放 `navidog-db`。
- **token**：Navicat 的实测数值只写在 `crates/navidog-ui/src/tokens.rs`（`Pt(n).rems()`，基准 13）和 `themes/navicat.json`，新 token 在 `docs/design/02-design-tokens.md` 登记来源。界面代码用具名 token 和 `palette(cx)`；必须写裸值时在同一行或上一行注明 `// raw-px: <理由>`。
- **图标**：`lucide(name)` 只能取到 `navidog-ui/src/assets.rs` 中 `ExtraIcons` 列出的图标和 gpui-kit 默认的 101 个，缺失的图标只在日志里留下加载错误、界面上显示为空白，新图标先加进列表。`every_icon_used_in_the_sources_loads` 测试会扫描源码中 `lucide("…")` 和 `button(id, "…", …)` 的字面量；运行时拼出的图标名（如 `kind_icon`）要自己核对。
- **连接配置文件**：`Connection` 顶层的 `general`、`advanced`、`ssl`、`ssh` 永远是当前激活的配置文件，其余配置文件在 `profiles` 里，切换时整体交换。按连接处理秘密字段的代码（重新加密、复制连接……）必须同时覆盖 `profiles`，参照 `ConnectionsFile::reencrypt`。
- **文案**：每个 crate 的 `locales/*.yml` 同时写 `en` 和 `zh-CN`，中文用词照 Navicat（`reference/help-text/`）。
- **单行文本**：对象名、字段值、属性、选项卡标题等显示数据或用户输入的文本，一律用 `navidog_ui::skin::truncated`：不换行，超出部分显示省略号，悬停时显示全文。它所在的 flex 容器要加 `min_w_0`，否则文本会撑开容器而不会被截断。网格单元格例外：只截断，不加悬停提示，因为每个单元格一个命中区域会拖慢滚动。
- **选项卡**：对象窗格中的面板在 `on_added_to`、`set_active`、`on_removed` 里调用 `navidog_ui::tabs` 的同名函数，`title()` 返回 `tabs::title(...)`（`tab_name` 保持 `None`），这样关闭、右键菜单、⌘W、在新窗口中打开才对它生效。`on_removed` 里 `tabs::removed` 返回 `true` 时，面板只是移到新窗口，不能关闭会话或丢弃修改。
- **注释**：代码注释用英文；改动一个文件时，把该文件里的中文注释顺手改成英文。`docs/` 和 `CONTEXT.md` 用中文。
- **数据库调用**：驱动在 `navidog_db::runtime` 上执行，界面用 `Sessions::run_rel`（元数据会话）或 `sessions::run_on`（自己持有的会话，表查看器这类编辑器经 `Sessions::new_session` 获得），在 `cx.spawn` 中等待结果。
- 组件能力不够时在外层补（delegate、自定义渲染），再不行基于 `gpui-base` 自建；gpui-kit 不 fork，`gpui-pre-*` 不单独覆盖。依赖不引入 GPL。
- **CI 脚本**：工作流调用的 shell 脚本放 `scripts/`（如 `scripts/upload_asset.sh`），不要放进 `.github/workflows/`。

## gpui-kit 0.7.3 已知的坑

- 切换外观时，只有带 `highlight` 的主题才会替换编辑器配色，所以 `themes/navicat.json` 的两套主题都要写全 `highlight`（`editor.*` 和 `syntax`）。没开 tree-sitter 特性时，gpui-component 用的高亮器存根不认 `editor.*` 键，测试主题的 crate 要开 `tree-sitter-sql`。
- `DataTable` 只在按下和抬起是同一格时发出 `SelectCell`；单元格不进测试元素注册表，UI 测试用 `click_at(("row", n), 偏移)` 点击。
- `DataTable` 把表头和表体包在 `context_menu` 里。0.7.3 里这个菜单是空的、也不绘制，仍会在 `defer` 里把焦点交给 `PopupMenu`。右键监听注册在子元素之后，冒泡时先跑，子元素里的 `stop_propagation` 赶不上。自己的右键菜单写在接到点击的元素上：`capture_any_mouse_down` 里对右键 `stop_propagation`，先 `focus` 挂着 `on_action` 的元素，再 `NativeMenu::show`。表头在 `crates/navidog-grid/src/grid/delegate.rs`。单元格用 `emit(RightClickedCell)`，这个效果排在浮层抢焦点之后，处理时把焦点交回表格。排序图标是 DataTable 画在 `render_th` 旁边的，不在这份捕获里。样式树以前同样用空菜单抢焦点；导航器行用基础树，右键时先聚焦导航器再弹出菜单。
- `Tree` 和 `DataTable` 的行由 `TreeState` / `TableState` 自己的视图渲染。只影响行外观的状态（重命名框、编辑中的单元格）改变后，要通知这个状态实体，只通知外层面板不会重绘行。
- 测试元素注册表只登记按钮、列表项等元素，普通容器的 id 用 `find` 找不到。
- headless 测试中 `dispatch_action` 之前要先 `render_frame` 并聚焦目标元素；`RadioGroup` 的子项 id 是 `ElementId::Integer(序号)`。
- headless 测试（`TestAppContext`、`HeadlessAppContext`）的调度器只在 `advance_clock` 时触发定时器。编辑器的文本超过 256 KB 或同步解析超过 2ms 时改在后台解析，前面有 150ms 防抖；不推进时钟就一直没有语法高亮，量出来的帧时间也偏低。防抖、自动保存这类定时逻辑同样要推进时钟。
- 测试里等真实数据库结果（`Sessions::run_rel`、`navidog_db::runtime::spawn` 的 `JoinHandle`）时，测试开头要 `cx.executor().allow_parking()`，否则结果从 `navidog-db` 线程唤醒前台任务的一刻被调度器记成 nondeterminism，测试结束时报 “Detected activity on thread … Your test is not deterministic”；前台任务先被 poll 过、后台任务后完成时才触发，所以表现为偶发。
- 同一个 `update_window` 闭包里的 `notify` 要到闭包结束才生效。会改变界面的命令放在单独的 `update_window` 里派发，接着 `run_until_parked()`，然后再输入文字或按键。
- 编辑器自带的键（`Input` context 里的 ⌘.、⌘F、Tab、Esc、回车）在外层 key context 里绑不过它：改绑写成后代谓词，如 `"QueryEditor > Input"`；要在编辑器处理之前截获 Tab、回车、Esc、Copy，用外层元素的 `capture_action`。
- `EditorState::set_scroll_offset` 不夹住范围，滚过末尾显示一片空白。滚到最后一行按行数、行高和视口高度自己算（`navidog_query::history::scroll_to_end`）。
- 对话框内容比对话框本身宽时，超出部分的点击落在遮罩上，会直接关闭对话框；窄对话框的按钮行要放得下（用图标按钮或分两行）。
- `window.click(id)` 拿当前帧的元素位置、对下一帧的命中框做判断，而对话框入场是走真实时钟的滑动（`advance_clock` 推不动它）。慢机器上元素在两次渲染之间移开，点击就落到相邻元素或遮罩上，同一份代码时好时坏。打开对话框后再点击的测试在初始化时 `cx.set_reduce_motion(true)`，让入场停在第一帧。
- `window.click` 总是发送空修饰键。⌘ 点按要派发带 `Modifiers::command()` 的鼠标事件，发完后把修饰键复位，示例见 `crates/navidog-navigator/tests/navigator.rs` 的 `command_click`。换 gpui-kit 版本后要跑全量测试。

## 验证

1. 运行 progress.md 第 5 节列出的全部检查。
   - 实库测试先执行 `docker compose up -d --wait`，测完执行 `docker compose down`。
   - 在 Cursor 沙箱中，`scripts/check-raw-px.sh`、`clippy` 和截图测试需要在沙箱外运行；沙箱会设置 `CARGO_TARGET_DIR`，沙箱外运行时加 `env -u CARGO_TARGET_DIR`，否则构建产物会写到沙箱缓存。
2. 改动界面时运行截图测试，逐张查看 `target/snapshots/` 下的浅色和深色截图。要给人看的图放进 PR 描述；测试截图留在本地，不提交进 git。
3. 每段不简单的逻辑至少留下一个可运行的检查（单元测试、headless UI 测试或实库测试）。

## 收尾

- 更新 progress.md：任务状态、功能状态及其“验证”列、临时偏差与遗留；同步勾选模块规格的验收清单。
- PR 的验收记录放 `docs/pr/acceptance-<主题>.md`；`docs/progress.md` 用 `pr/…` 链接，`docs/design/` 里的规格用 `../pr/…`。
- 提交到当前里程碑分支（`mN-…`）。推送和开 PR 要先征得用户同意。
