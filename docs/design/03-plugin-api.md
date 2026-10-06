# 插件 API

navidog 的插件是 [gpui-shell](https://gpui-kit.com/zh-CN/shell) 应用：一个目录，里面有 JavaScript 源码、`gpui-shell.json` 和 navidog 自己的 `navidog.json`。本文定义注册表、插件目录结构、授权、HostModule `navidog` 和 JS 驱动协议。实现安排在 M5，注册表在 M1 就建立（决策见 [00-overview.md](00-overview.md#3-决策记录)）。

## 1. 设计约束（来自 gpui-shell）

| 约束 | 后果 |
| --- | --- |
| 脚本默认没有任何能力，文件、网络、剪贴板都要由宿主授予；授予内容在 View 加载时冻结。 | 授权必须在加载插件之前完成，修改授权后要重新加载插件。 |
| `gpui-shell.json` 中出现未知字段时，整个 manifest 判定为无效。 | navidog 的扩展声明不能写进 `gpui-shell.json`，单独放在 `navidog.json`。 |
| HostModule 边界上只能传纯数据（null、布尔、数字、字符串、数组、对象），宿主拿不到脚本函数的句柄。 | 插件不能把回调函数注册给宿主。 |
| host function 禁止回调进脚本引擎。 | 宿主要通知插件（命令被触发、驱动请求等），只能由插件拉取：`await next_event()`。 |
| 每个插件有独立的 `Policy`，包含自己的 capabilities 和 HostModule 注册表（`Policy::with_host_module`）。 | navidog 自有的能力（读取连接列表、执行 SQL 等）按插件分别注册：没有授权的函数不会出现在该插件的 `navidog` 模块里。 |
| 脚本 View 需要挂在 `ShellRoot` 之下，`window.open_dialog` 等调用才能生效。 | 插件面板在 Dock Panel 内部包一层 `ShellRoot`，M5 需要先验证这样做能否与宿主的 `Root` 共存。 |

## 2. 注册表（`navidog-core`，M1）

内置功能和插件功能都通过同一套注册表注册。每个条目都带有 `source` 字段，取值为 `Builtin` 或 `Plugin(PluginId)`；停用插件时，按 source 批量移除它注册的条目。

```rust
pub enum Source {
    Builtin,
    Plugin(PluginId),
}

pub struct Registry<K, V> { /* 按 K 索引，保持注册顺序 */ }

impl<K: Eq + Hash + Clone, V> Registry<K, V> {
    pub fn register(&mut self, key: K, source: Source, value: V) -> Result<(), RegistryError>;
    pub fn unregister_source(&mut self, source: &Source);
    pub fn get(&self, key: &K) -> Option<&V>;
    pub fn iter(&self) -> impl Iterator<Item = (&K, &Source, &V)>;
}
```

| 注册表 | 键 | 值 | 内置条目示例 |
| --- | --- | --- | --- |
| `CommandRegistry` | `CommandId`（如 `connection.new`） | 标题（i18n 键）、图标、默认快捷键、可用条件、菜单与工具栏位置 | 新建连接、新建查询、刷新 |
| `DriverRegistry` | `DriverId`（如 `mysql`） | 驱动元数据（名称、类别、图标、默认端口、表单字段集合）和 `Arc<dyn Driver>` | mysql、mariadb、postgres、sqlite |
| `FormatRegistry` | `FormatId`（如 `csv`） | 导入导出格式元数据和实现 | csv、json、sql |
| `CellFormatterRegistry` | `FormatterId` | 声明式格式化规则 | 时间戳显示为日期 |
| `PanelRegistry` | `PanelId` | 面板元数据（标题、默认停靠位置、尺寸）和构造函数 | 导航窗格、信息窗格 |

注册表是 `navidog-core` 中的 GPUI `Global`；修改时调用 `cx.refresh_windows()`，菜单栏通过 `cx.set_menus` 重建。

## 3. 插件目录

```
~/Library/Application Support/navidog/plugins/
  com.example.clickhouse/
    gpui-shell.json      # gpui-shell 的 manifest：id、name、entry、capabilities
    navidog.json         # navidog 的扩展声明
    main.js              # 服务入口：init 中启动事件循环
    panels/explorer.js   # 每个面板一个入口，default 导出 View
```

### 3.1 `gpui-shell.json`

格式完全由 gpui-shell 定义：`id`（反向域名）、`name`、`version`、`shell-version`、`entry`、`dependencies`、`capabilities`。navidog 不添加任何字段。

### 3.2 `navidog.json`

```json
{
  "api": 1,
  "permissions": ["connections.read", "query.execute"],
  "commands": [
    { "id": "copy-as-markdown", "title": "Copy as Markdown", "title_zh": "复制为 Markdown",
      "menu": "grid.context", "shortcut": null }
  ],
  "formats": [
    { "id": "parquet", "name": "Parquet", "extensions": ["parquet"], "import": false, "export": true }
  ],
  "cell_formatters": [
    { "id": "unix-ts", "match": { "type": ["int", "bigint"], "column": "(?i)_at$" },
      "display": { "kind": "unix_timestamp", "unit": "s", "format": "%Y-%m-%d %H:%M:%S" } }
  ],
  "drivers": [
    { "id": "clickhouse", "name": "ClickHouse", "family": "relational", "icon": "icons/ch.svg",
      "default_port": 8123, "fields": ["host", "port", "user", "password", "database"] }
  ],
  "panels": [
    { "id": "explorer", "title": "Explorer", "entry": "panels/explorer.js",
      "placement": "right", "size": 280 }
  ]
}
```

- `api`：插件 API 的主版本。宿主不支持该版本时拒绝加载，并说明原因。
- `permissions`：navidog 自有能力的申请（见第 4 节）。文件、网络等系统能力仍然写在 `gpui-shell.json` 的 `capabilities` 中。
- 所有 `id` 都在插件内唯一；注册到全局时加上插件前缀，例如 `com.example.clickhouse/copy-as-markdown`。

## 4. 授权

首次启用插件时，弹出授权对话框，列出两类申请：

1. **系统能力**（`gpui-shell.json` 的 `capabilities`）：文件读写目录、网络主机、HTTP 规则、剪贴板、存储。原样展示，`${pluginDir}`、`${dataDir}` 替换为实际路径。
2. **navidog 能力**（`navidog.json` 的 `permissions`）：

| 能力 | 允许插件做什么 |
| --- | --- |
| `connections.read` | 读取连接列表的元数据（名称、类型、主机、颜色），不包含密码 |
| `query.execute` | 在用户已打开的连接上执行 SQL；每次执行写入历史日志并标注插件 ID |
| `grid.read` | 读取当前结果网格的选中数据 |
| `editor.write` | 向当前查询编辑器插入文本 |

用户可以逐项取消勾选。授权结果保存在 `plugins.json`，可以在偏好设置的插件页撤销。授权变化后插件会重新加载。

JS 驱动需要的网络能力也来自 `gpui-shell.json`，例如 `network.hosts` 或 `network.http`。驱动只能拿到使用该驱动的连接的凭据，并且只在 `driver.connect` 请求中拿到。

## 5. HostModule `navidog`

宿主按插件分别构造 `HostModule::new("navidog")`，只注册已授权的函数，并通过 `declarations` 提供类型，让 gpui-shell 生成的 `gpui-kit.d.ts` 中包含 `declare module "navidog"`。

```ts
// 始终可用
export function api_version(): number;
export function locale(): string;                       // "zh-CN" | "en"
export function next_event(): Promise<NavidogEvent>;   // 拉取下一个宿主事件
export function respond(request_id: string, result: Reply): void;
export function respond_chunk(request_id: string, chunk: ResultChunk): void;
export function notify(level: "info" | "success" | "warning" | "error", message: string): void;

// connections.read
export function list_connections(): ConnectionInfo[];

// query.execute
export function execute(connection_id: string, database: string | null, sql: string): Promise<ResultSet[]>;

// grid.read
export function grid_selection(): ResultSet | null;

// editor.write
export function insert_text(text: string): void;
```

### 5.1 事件循环

插件入口的 `init` 中启动一个循环，在循环里拉取事件。宿主为每个插件维护一个事件队列；插件停用时，未完成的 `next_event` 以错误结束，循环随之退出。

```js
import { next_event, respond } from "navidog";

export default class Service extends View {
  init(_props, cx) {
    cx.spawn(async () => {
      for (;;) {
        const event = await next_event();
        switch (event.kind) {
          case "command":
            /* event.command_id, event.context */
            break;
          case "driver.request":
            respond(event.request_id, await handleDriver(event));
            break;
          case "export":
            respond(event.request_id, await exportRows(event));
            break;
        }
      }
    });
  }
  render() { return div(); }
}
```

| `event.kind` | 触发时机 | 需要回复 |
| --- | --- | --- |
| `command` | 用户触发了插件注册的命令 | 否 |
| `driver.request` | JS 驱动收到宿主请求（见第 6 节） | 是 |
| `export` | 用户选择插件格式导出，事件中带有按批次提供的行数据 | 是 |
| `import` | 用户选择插件格式导入，插件以 `respond_chunk` 回传行数据 | 是 |
| `cancel` | 宿主取消一个未完成的请求 | 否 |

宿主为每个请求设置超时（默认 30 秒，驱动请求使用连接设置中的超时），超时后向调用方报错，并给插件发送 `cancel` 事件。

## 6. JS 驱动协议

宿主侧的 `JsDriver` 实现 `navidog-db` 的 `Driver` 和 `Relational` trait，把每次调用转换成 `driver.request` 事件。

| `op` | 参数 | 回复 |
| --- | --- | --- |
| `connect` | `connection`：表单字段值，包括凭据 | `{ session_id }` |
| `disconnect` | `session_id` | `{}` |
| `ping` | `session_id` | `{}` |
| `server_version` | `session_id` | `{ version }` |
| `list_databases` | `session_id` | `{ databases: string[] }` |
| `list_schemas` | `session_id, database` | `{ schemas: string[] }` |
| `list_objects` | `session_id, database, schema, kind` | `{ objects: ObjectInfo[] }` |
| `execute` | `session_id, sql` | 先用若干个 `respond_chunk` 回传结果，最后用 `respond` 收尾 |
| `cancel` | `session_id` | `{}` |

结果数据的格式：

```ts
type ResultChunk = {
  columns?: { name: string; type: string; nullable?: boolean }[]; // 仅第一个 chunk 携带
  rows: (null | boolean | number | string)[][];                    // 二进制以 base64 字符串传输
};
type Reply = { ok: true; data?: unknown } | { ok: false; error: string; code?: string };
```

数值超出 JS 安全整数范围时，必须以字符串传输。宿主按列类型解析成网格值。

## 7. 单元格格式化器

渲染期间不能调用 JS，所以格式化器是声明式的：`navidog.json` 中的 `cell_formatters` 由 Rust 解析并执行。

- `match`：按列类型列表、列名正则、值正则匹配，条件之间是“且”的关系。
- `display.kind`：取值为 `unix_timestamp`、`json_pretty`、`enum_map`（值到标签的映射）、`number`（小数位、千分位）、`template`（`{value}` 占位符）、`color`（按值着色，颜色只能用 token 名）。

如果需要更复杂的转换，插件可以注册一个命令，由命令生成派生列，而不是使用格式化器。

## 8. 插件面板

- `navidog.json` 中的每个面板，宿主用 `runtime.load_app(plugin_dir, entry)` 加载，再 `instantiate` 成 View 对象，包装成 `ScriptView`，放进实现了 `gpui-component` `Panel` 的 `PluginPanel` 中，然后按 `placement`（left、right、bottom、center）和 `size` 加入 `DockArea`。
- 面板标识为 `plugin:<插件 id>/<面板 id>`，用于 Dock 布局的持久化。插件未加载时，布局中对应的位置显示为占位面板。
- 宿主状态（连接列表、主题、语言）发生变化时，宿主调用 `view.refresh(cx)`，让脚本重新执行 `render`。

## 9. 生命周期

1. **发现**：启动时扫描插件目录，读取两份 manifest，不执行任何代码。
2. **校验**：检查 `api` 版本、id 格式、`shell-version`，以及 `navidog.json` 的 schema。
3. **授权**：首次启用时弹出授权对话框，之后读取 `plugins.json`。
4. **加载**：为插件构造 `Policy`（capabilities 加 `navidog` HostModule），加载服务入口，注册 manifest 中声明的命令、格式、格式化器、驱动和面板。
5. **停用或卸载**：从所有注册表移除该插件的条目（`unregister_source`），关闭它的面板，结束事件队列。
6. **开发模式**：debug 构建中对插件目录调用 `runtime.watch`，文件变化时热重载。

## 10. 待 M5 验证的风险

- 插件面板内包一层 `ShellRoot` 能否与宿主的 `Root` 共存，以及对话框是否只覆盖在面板范围内。
- 服务入口的 View 需要一个活的 `Window` 才能实例化：确认在主窗口中实例化但不渲染时，它的事件循环是否正常运行。
- QuickJS 运行时会让二进制增加约 13.5 MiB：确认 release 构建体积可以接受。
