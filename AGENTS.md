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
- **注释**：代码注释用英文；改动一个文件时，把该文件里的中文注释顺手改成英文。`docs/` 和 `CONTEXT.md` 用中文。
- **数据库调用**：驱动在 `navidog_db::runtime` 上执行，界面用 `Sessions::run_rel`（元数据会话）或 `sessions::run_on`（自己持有的会话，表查看器这类编辑器经 `Sessions::new_session` 获得），在 `cx.spawn` 中等待结果。
- 组件能力不够时在外层补（delegate、自定义渲染），再不行基于 `gpui-base` 自建；gpui-kit 不 fork，`gpui-pre-*` 不单独覆盖。依赖不引入 GPL。

## gpui-kit 0.7.1 已知的坑

- 切换外观时，只有带 `highlight` 的主题才会替换编辑器配色，所以 `themes/navicat.json` 的两套主题都要写全 `highlight`（`editor.*` 和 `syntax`）。没开 tree-sitter 特性时，gpui-component 用的高亮器存根不认 `editor.*` 键，测试主题的 crate 要开 `tree-sitter-sql`。
- `DataTable` 只在按下和抬起是同一格时发出 `SelectCell`；单元格不进测试元素注册表，UI 测试用 `click_at(("row", n), 偏移)` 点击。
- 测试元素注册表只登记按钮、列表项等元素，普通容器的 id 用 `find` 找不到。
- headless 测试中 `dispatch_action` 之前要先 `render_frame` 并聚焦目标元素；`RadioGroup` 的子项 id 是 `ElementId::Integer(序号)`。

## 验证

1. 运行 progress.md 第 5 节列出的全部检查。
   - 实库测试先执行 `docker compose up -d --wait`，测完执行 `docker compose down`。
   - 在 Cursor 沙箱中，`scripts/check-raw-px.sh`、`clippy` 和截图测试需要在沙箱外运行；沙箱会设置 `CARGO_TARGET_DIR`，沙箱外运行时加 `env -u CARGO_TARGET_DIR`，否则构建产物会写到沙箱缓存。
2. 改动界面时运行截图测试，逐张查看 `target/snapshots/` 下的浅色和深色截图。
3. 每段不简单的逻辑至少留下一个可运行的检查（单元测试、headless UI 测试或实库测试）。

## 收尾

- 更新 progress.md：任务状态、功能状态及其“验证”列、临时偏差与遗留；同步勾选模块规格的验收清单。
- 提交到当前里程碑分支（`mN-…`）。推送和开 PR 要先征得用户同意。
