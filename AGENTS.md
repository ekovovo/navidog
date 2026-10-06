# navidog Agent 指南

navidog 用 gpui-kit 0.7 复刻 Navicat Premium 17（macOS）。决策、架构和工程规则的唯一来源是 [docs/design/00-overview.md](docs/design/00-overview.md)，术语以 [CONTEXT.md](CONTEXT.md) 为准；本文只补充开发流程和代码里看不出来的坑。

## 开工

1. 读 [docs/progress.md](docs/progress.md)，确定当前里程碑和要做的任务。
2. 读该任务对应的模块规格（`docs/design/1x-*.md`、`2x-*.md`……）和它引用的功能编号（`F-…`，见 `01-feature-inventory.md`）。规格缺少尺寸、默认值或行为时，先补规格；规格和代码分开提交。
3. Navicat 的尺寸、文案、行为从本地 `reference/` 查（导出命令见 00-overview 第 2 节）。`reference/` 只在本地使用，内容不进仓库；Navicat 的图标和 logo 不提取。

## 写代码

- **依赖方向**：feature crate（`navidog-connection`、`navidog-navigator`、`navidog-grid`……）之间不互相依赖。跨 feature 的命令放 `navidog_core::actions`，会话放 `navidog_db::sessions`，由外壳 `crates/navidog` 组合面板（按对象类型选编辑器在 `src/editors.rs`）。
- **token**：Navicat 的实测数值只写在 `crates/navidog-ui/src/tokens.rs`（`Pt(n).rems()`，基准 13）和 `themes/navicat.json`，新 token 在 `docs/design/02-design-tokens.md` 登记来源。界面代码用具名 token 和 `palette(cx)`；必须写裸值时在同一行或上一行注明 `// raw-px: <理由>`。
- **图标**：`lucide(name)` 只能取到 `navidog-ui/src/assets.rs` 中 `ExtraIcons` 列出的图标和 gpui-kit 默认的 101 个，缺失的图标只在日志里留下加载错误、界面上显示为空白，新图标先加进列表。
- **文案**：每个 crate 的 `locales/*.yml` 同时写 `en` 和 `zh-CN`，中文用词照 Navicat（`reference/help-text/`）。
- **单行文本**：对象名、字段值、属性、选项卡标题等显示数据或用户输入的文本，一律用 `navidog_ui::skin::truncated`：不换行，超出部分显示省略号，悬停时显示全文。它所在的 flex 容器要加 `min_w_0`，否则文本会撑开容器而不会被截断。网格单元格例外：只截断，不加悬停提示，因为每个单元格一个命中区域会拖慢滚动。
- **选项卡**：对象窗格中的面板在 `on_added_to`、`set_active`、`on_removed` 里调用 `navidog_ui::tabs` 的同名函数，`title()` 返回 `tabs::title(...)`（`tab_name` 保持 `None`），这样关闭、右键菜单、⌘W、在新窗口中打开才对它生效。`on_removed` 里 `tabs::removed` 返回 `true` 时，面板只是移到新窗口，不能关闭会话或丢弃修改。
- **注释**：代码注释用英文；改动一个文件时，把该文件里的中文注释顺手改成英文。`docs/` 和 `CONTEXT.md` 用中文。
- **数据库调用**：驱动在 `navidog_db::runtime` 上执行，界面用 `Sessions::run_rel`（元数据会话）或 `sessions::run_on`（自己持有的会话，表查看器这类编辑器经 `Sessions::new_session` 获得），在 `cx.spawn` 中等待结果。
- 组件能力不够时在外层补（delegate、自定义渲染），再不行基于 `gpui-base` 自建；gpui-kit 不 fork，`gpui-pre-*` 不单独覆盖。依赖不引入 GPL。

## gpui-kit 0.7.1 已知的坑

- 切换外观时，只有带 `highlight` 的主题才会替换编辑器配色，所以 `themes/navicat.json` 的两套主题都要写全 `highlight`（`editor.*` 和 `syntax`）。没开 tree-sitter 特性时，gpui-component 用的高亮器存根不认 `editor.*` 键，测试主题的 crate 要开 `tree-sitter-sql`。
- `DataTable` 只在按下和抬起是同一格时发出 `SelectCell`；单元格不进测试元素注册表，UI 测试用 `click_at(("row", n), 偏移)` 点击。
- `Tree` 和 `DataTable` 的行由 `TreeState` / `TableState` 自己的视图渲染。只影响行外观的状态（重命名框、编辑中的单元格）改变后，要通知这个状态实体，只通知外层面板不会重绘行。
- 测试元素注册表只登记按钮、列表项等元素，普通容器的 id 用 `find` 找不到。
- headless 测试中 `dispatch_action` 之前要先 `render_frame` 并聚焦目标元素；`RadioGroup` 的子项 id 是 `ElementId::Integer(序号)`。
- 同一个 `update_window` 闭包里的 `notify` 要到闭包结束才生效。会改变界面的命令放在单独的 `update_window` 里派发，接着 `run_until_parked()`，然后再输入文字或按键。
- 需要带修饰键的点击（⌘ 点按）时，用 `window.dispatch_event` 发送带 `modifiers` 的 `MouseDownEvent` / `MouseUpEvent`，示例见 `crates/navidog-navigator/tests/navigator.rs`。

## 验证

1. 运行 progress.md 第 5 节列出的全部检查。
   - 实库测试先执行 `docker compose up -d --wait`，测完执行 `docker compose down`。
   - 在 Cursor 沙箱中，`scripts/check-raw-px.sh`、`clippy` 和截图测试需要在沙箱外运行；沙箱会设置 `CARGO_TARGET_DIR`，沙箱外运行时加 `env -u CARGO_TARGET_DIR`，否则构建产物会写到沙箱缓存。
2. 改动界面时运行截图测试，逐张查看 `target/snapshots/` 下的浅色和深色截图。
3. 每段不简单的逻辑至少留下一个可运行的检查（单元测试、headless UI 测试或实库测试）。

## 收尾

- 更新 progress.md：任务状态、功能状态及其“验证”列、临时偏差与遗留；同步勾选模块规格的验收清单。
- 提交到当前里程碑分支（`mN-…`）。推送和开 PR 要先征得用户同意。
