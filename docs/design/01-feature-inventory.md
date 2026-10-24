# 全量功能清单

本清单收录 Navicat Premium 17.3.5 中适用于 MySQL/MariaDB、PostgreSQL、SQLite 的全部功能，外加 AI 助手和替代 Navicat Cloud 的同步功能。只属于其他数据库的功能见 [db/](db/)。

**怎么读这份清单**
- **编号**：`F-<领域>-<序号>`。编号一经分配不再改变；功能被删除时保留编号，并标记“已取消”。
- **帮助页**：指 `zh-Hans.lproj/Navicat Help/` 下的文件名，是行为的权威来源。
- **里程碑**：对应 [00-overview.md](00-overview.md#6-里程碑)。
- **方言标记**：`[My]` 只适用于 MySQL/MariaDB，`[PG]` 只适用于 PostgreSQL，`[Lite]` 只适用于 SQLite；不标即三者都适用。
- **验收要点**：只列关键检查项；完整的验收清单写在各里程碑的详细规格里。

## APP 应用外壳

| 编号 | 功能 | 摘要与验收要点 | 帮助页 | 里程碑 |
| --- | --- | --- | --- | --- |
| F-APP-001 | 主窗口布局 | 由主工具栏、导航窗格、对象窗格、信息窗格、状态栏组成，默认尺寸和分栏宽度与基线一致。Dock 允许拖拽停靠（已知偏差）。 | main_window | M1 |
| F-APP-002 | 主工具栏 | Expanded 样式，大图标加文字；按钮集合随所选连接类型变化；可通过“查看 > 显示工具栏”隐藏（⌥⌘T）。 | main_window | M1 |
| F-APP-003 | 菜单栏 | Navicat、文件、编辑、查看、收藏夹、工具、窗口、帮助各菜单结构与基线一致，快捷键与 `hot_keys` 一致。 | hot_keys | M1 |
| F-APP-004 | 状态栏 | 24pt 高，显示当前窗口的状态信息；可通过“查看 > 显示状态栏”隐藏。 | main_window | M1 |
| F-APP-005 | 编辑器选项卡 | 在对象窗格中打开选项卡；⌘} / ⌘{ 切换，⌘W 关闭；选项卡可弹出为新窗口；可设置弹出内容在新选项卡或新窗口中打开。 | main_window, preferences_tab | M1（框架）、M2（内容） |
| F-APP-006 | 显示或隐藏窗格 | ⌘1 切换导航窗格，⌘2 切换信息窗格；状态持久化。 | hot_keys | M1 |
| F-APP-007 | 窗口与布局持久化 | 记住窗口位置尺寸、Dock 布局、各窗格显示状态，重启后恢复。 | — | M1 |
| F-APP-008 | 全屏与专注模式 | ⌃⌘F 全屏；专注模式隐藏除当前编辑器外的界面，适用于数据编辑器、设计器、查询等。 | focus | M1（全屏）、M2（专注模式） |
| F-APP-009 | 浅色与深色外观 | 跟随系统，也可在偏好设置中固定；两套主题 token 完整。 | dark_theme | M1 |
| F-APP-010 | 界面语言 | 简体中文、英文，可在偏好设置中切换，切换后刷新所有窗口。 | preferences_gen | M1 |
| F-APP-011 | 收藏夹 | 把对象或选项卡加入收藏夹（⌥⇧1-9）并快速打开（⌃⇧1-9）；收藏夹菜单。 | favorites | M2 |
| F-APP-012 | 启动画面 | 启动时打开的选项卡列表（按住 Control 点按选项卡加入）。 | main_window, preferences_tab | M2 |
| F-APP-013 | 打开最近使用过的 | 文件菜单中列出最近打开的对象，可清除。 | — | M2 |
| F-APP-014 | 使用 navidog URI 打开 | ⌘U 粘贴 URI 打开对象，支持连接映射（自动匹配、现有连接、新建连接）。URI 格式由 navidog 自定，不兼容 Navicat URI。 | share_uri | M2 |
| F-APP-015 | 通过 URI 共享 | 在信息窗格“常规”页生成对象的 URI，可选隐藏主机信息。 | share_uri | M2 |
| F-APP-016 | 自动恢复 | 定期保存未保存的查询和设计器内容，异常退出后提示恢复。 | preferences_auto_recovery | M3 |
| F-APP-017 | 日志文件 | `tracing` 应用日志、SQL 执行日志、命令行日志，存放在 `~/Library/Logs/navidog/`。 | log_files | M1（应用日志）、M3（SQL 日志） |
| F-APP-018 | 关于与检查更新 | “关于 navidog”窗口。“检查更新”读取 GitHub Releases 上的 `latest.json`（先 nightly，没有再试最新稳定版；`releases` 是各版本列表），有新版本时在应用内下载并校验 SHA-256 后安装。不含预发布时跳过预发布和 nightly。没有发布版本和网络失败分开提示。菜单回调没有活动窗口时用最前面的窗口。偏好设置可包含预发布和 nightly。启动时最多每两天检查一次，可关掉。 | about | M1、M15 |
| F-APP-019 | 欢迎界面 | 首次启动的引导窗口，没有连接时对象窗格显示“点击工具栏中的‘连接’开始”。 | — | M1 |

## PREF 偏好设置

| 编号 | 功能 | 摘要与验收要点 | 帮助页 | 里程碑 |
| --- | --- | --- | --- | --- |
| F-PREF-001 | 偏好设置窗口 | ⌘, 打开，左侧分类列表，右侧设置页；修改即时生效并持久化。 | preferences_settings | M1 |
| F-PREF-002 | 常规 | 主题（Navidog、Default）、外观、语言、启动行为、导航窗格显示选项（显示模式下对象、显示表下对象）等。 | preferences_gen | M1 |
| F-PREF-003 | 选项卡 | 弹出内容在新选项卡还是新窗口中打开、启动画面、选项卡关闭行为。 | preferences_tab | M2 |
| F-PREF-004 | 编辑器 | 字体、字号、行号、代码折叠、自动换行、制表符宽度等。 | preferences_editor | M3 |
| F-PREF-005 | 记录 | 每页记录数（默认 1000）、网格字体、显示主键提示、自动开始事务等。 | preferences_record | M2 |
| F-PREF-006 | 代码补全 | 使用代码补全、自动更新代码补全的信息、包括系统对象、自动选择第一个建议项目、清除代码补全的信息。 | preferences_code_completion | M3 |
| F-PREF-007 | AI | AI 提供商、模型、API Key（加密存储）。 | preferences_ai | M11 |
| F-PREF-008 | 自动恢复 | 自动保存已修改的查询的间隔（每 15 秒到每 5 分钟，或永不）；模型和 BI 工作区的间隔随 M7、M10 加入。 | preferences_auto_recovery | M3 |
| F-PREF-009 | 环境 | 外部工具路径，例如 `mysqldump`、`pg_dump`。 | preferences_env | M9 |
| F-PREF-010 | 高级 | 设置位置（配置目录）、日志选项（日志路径、启用诊断日志）、主密码。 | preferences_adv | M1、M3（日志选项） |

## CONN 连接

| 编号 | 功能 | 摘要与验收要点 | 帮助页 | 里程碑 |
| --- | --- | --- | --- | --- |
| F-CONN-001 | 新建连接：选择类型 | 点击工具栏“连接”或“文件 > 新建 > 连接”，打开类型选择窗口（左侧分类，右侧图标网格，支持搜索），选择后进入连接表单。 | connection | M1 |
| F-CONN-002 | 连接表单：常规 | 按数据库类型显示字段：MySQL 为连接名称、添加到、主机、端口、用户名、密码、保存密码；PostgreSQL 另有初始数据库；SQLite 为类型（现有文件、新建 SQLite 3）、数据库文件、用户名、密码。尺寸与 nib 一致。 | con_gen_settings, con_gen_settings_postgresql, con_gen_settings_sqlite | M1 |
| F-CONN-003 | 连接表单：高级 | 设置位置、客户端字符集、编码、保持连接间隔、连接/读取/写入超时、使用压缩、自动连接、使用套接字文件、限制连接会话、初始查询、连接参数；按方言显示不同字段。 | con_adv_settings | M1 |
| F-CONN-004 | 连接表单：数据库 | “使用自定义数据库列表”，勾选显示的数据库及“自动打开”，可添加隐藏数据库。SQLite 为附加数据库文件列表。 | con_db_settings | M1 |
| F-CONN-005 | 连接表单：SSL | MySQL：客户端密钥、客户端证书、CA 证书、验证服务器证书、指定密码；PostgreSQL：SSL 模式（allow、prefer、require、verify-ca、verify-full）、客户端密钥、客户端证书、根证书、证书撤消列表。SQLite 不显示该页。 | con_ssl_settings | M1 |
| F-CONN-006 | 连接表单：SSH | 使用 SSH 隧道；主机、端口、用户名；验证方法为密码、公钥、密码和公钥；私钥、通行短语；保存密码和通行短语。SQLite 不显示该页。 | con_ssh_settings | M1 |
| F-CONN-007 | 连接表单：HTTP 隧道 | 通过自写的服务端脚本转发；与 SSH 互斥。 | con_http_settings | M13 |
| F-CONN-008 | 连接表单：兼容性 [My] | 从“高级”页的“兼容性”按钮进入，设置 MySQL 兼容模式。 | con_compatibility_settings | M1 |
| F-CONN-009 | 连接表单：备注 | 连接的备注文本。 | — | M1 |
| F-CONN-010 | 测试连接 | 在表单中测试当前填写的参数（包含 SSL/SSH），成功或失败都给出提示，失败时显示服务器错误信息。 | connection | M1 |
| F-CONN-011 | 编辑、复制、删除连接 | 右键菜单“编辑连接”“复制连接”“删除连接”；删除前确认；编辑已打开的连接时提示重连。 | connection | M1 |
| F-CONN-012 | 打开、关闭连接 | 双击或右键“打开连接”；⇧⌘. 关闭连接、数据库或模式；打开失败给出错误对话框；自动连接的连接在启动时打开。 | navigation_pane, hot_keys | M1 |
| F-CONN-013 | 连接组 | 在“我的连接”下新建、重命名、删除组，拖放移动连接；“隐藏连接组”切换为扁平显示。 | virtual_group, navigation_pane | M1 |
| F-CONN-014 | 连接颜色 | 右键“颜色”设置标识色，显示在导航树和相关选项卡上。 | connection_color | M1 |
| F-CONN-015 | 管理连接窗口 | 集中管理“我的连接”：新建、编辑、标星、隐藏、删除、搜索连接信息并高亮匹配。 | con_manage | M2 |
| F-CONN-016 | 通过 URI 新建连接 | 在类型选择窗口粘贴 navidog URI 或标准数据库 URI（`mysql://`、`postgres://`），自动填写表单。 | connection | M2 |
| F-CONN-017 | 导出与导入连接 | “文件 > 导出连接/导入连接”，navidog 自有格式，可选包含密码（需确认）。 | — | M6 |
| F-CONN-018 | 导入 Navicat `.ncx` | 读取 Navicat 导出的连接文件并转换。 | — | M6 |
| F-CONN-019 | 重载 [My] | 右键“重载”，选择 FLUSH 选项执行，需要 RELOAD 权限。 | connection | M2 |
| F-CONN-020 | 密码存储 | 密码加密保存；可设置主密码；“保存密码”未勾选时每次连接弹出询问密码对话框。 | — | M1 |
| F-CONN-021 | 使用不同配置文件连接 | 同一连接保存多套参数配置，打开时选择。 | con_profile | M2 |

## NAV 导航窗格

| 编号 | 功能 | 摘要与验收要点 | 帮助页 | 里程碑 |
| --- | --- | --- | --- | --- |
| F-NAV-001 | 连接树 | 层级：我的连接 > 组 > 连接 > 数据库 >（模式 [PG]）> 对象分类（表、视图、函数、事件 [My]、查询、备份等）> 对象；虚拟化渲染；图标和缩进与基线一致。 | navigation_pane | M1（到数据库）、M2（到对象） |
| F-NAV-002 | 表下元素 | 偏好设置开启后，表下显示字段、索引、外键、触发器；可右键重命名字段，双击打开表设计器。 | navigation_pane | M2 |
| F-NAV-003 | 筛选 | 搜索框输入即筛选，匹配项的上级自动展开；右侧图标按连接类型、活动对象、标星快速筛选；“在顶部显示筛选”切换位置。 | navigation_pane, search_filter | M1（搜索）、M2（快速筛选） |
| F-NAV-004 | 仅显示活跃对象 | 只显示已打开的连接和数据库。 | — | M2 |
| F-NAV-005 | 扁平连接 | “查看 > 导航窗格 > 扁平连接”。 | — | M2 |
| F-NAV-006 | 右键菜单 | 每类节点的右键菜单（用 `NativeMenu`），菜单项与基线一致。 | navigation_pane | M1（连接、组）、M2（其余） |
| F-NAV-007 | 虚拟组 | 为连接、表、视图、函数、查询、备份、模型等对象建立逻辑分组，支持拖放；每个连接保存在其目录的 `.vgroups.json`。 | virtual_group | M2 |
| F-NAV-008 | 键盘操作 | 方向键移动、左右键折叠展开、回车打开、Delete 删除（需确认）、⌘R 刷新。 | hot_keys | M1 |

## OBJ 对象选项卡

| 编号 | 功能 | 摘要与验收要点 | 帮助页 | 里程碑 |
| --- | --- | --- | --- | --- |
| F-OBJ-001 | 对象列表视图 | 列出当前对象分类下的对象，支持图标或列表样式，双击打开。 | object_pane | M2 |
| F-OBJ-002 | 详细信息视图 | 以列显示名称和属性（行数、数据长度、引擎、创建时间、注释等），“选择列”配置每类对象显示的列，按列排序。 | object_pane | M2 |
| F-OBJ-003 | ER 图表视图 | 自动生成当前数据库的 ER 图；拖放字段新建外键；缩放、手型移动、自动布局、导出 PNG、纸张大小。 | object_pane | M7 |
| F-OBJ-004 | 对象工具栏 | 按对象分类显示打开、设计、新建、删除、导入、导出等按钮。 | object_pane | M2 |
| F-OBJ-005 | 对象组与扁平列表 | 列表中显示虚拟组；“扁平对象列表”取消分组。 | object_pane | M2 |
| F-OBJ-006 | 显示隐藏项和已删除项 | “显示隐藏的项目”“显示已删除的项目”。 | — | M2 |
| F-OBJ-007 | 表概况 | “显示表概况”，在对象列表中显示统计信息。 | — | M2 |
| F-OBJ-008 | 对象操作 | 新建、打开、设计、复制（结构或结构加数据）、重命名、删除、清空、截断表；⌘N、⌘D、⇧⌘D 快捷键。 | mysql_table, postgresql_table, sqlite_table | M2 |
| F-OBJ-009 | 维护 | MySQL：分析、检查、优化、修复表；PostgreSQL：VACUUM、ANALYZE、REINDEX；SQLite：VACUUM、ANALYZE、REINDEX、完整性检查。 | mysql_maintain, postgresql_maintain, sqlite_maintain | M4 |

## INFO 信息窗格

| 编号 | 功能 | 摘要与验收要点 | 帮助页 | 里程碑 |
| --- | --- | --- | --- | --- |
| F-INFO-001 | 常规 | 所选连接或对象的常规信息（服务器版本、对象属性、创建时间等），含“共享”按钮。 | information_pane | M2 |
| F-INFO-002 | DDL | 显示对象的 DDL，只读语法高亮，⌘F 搜索。 | information_pane | M2 |
| F-INFO-003 | 使用与被使用 | 显示对象依赖的对象和依赖它的对象。 | information_pane | M4 |
| F-INFO-004 | 预览 | 显示查询的 SQL 语句。 | information_pane | M3 |
| F-INFO-005 | 代码段 | 列出预设和自定义代码段，按标签、数据库类型筛选；拖到编辑器插入，双击查看和编辑。 | information_pane, query_code_snippet | M3 |
| F-INFO-006 | 标识符 | 列出当前数据库中所有表、视图、字段，供插入编辑器。 | information_pane | M3 |
| F-INFO-007 | 字段 | 在表查看器中显示所选字段的类型、注释等信息。 | information_pane, data_field_comment | M2 |
| F-INFO-008 | 成员、成员属于、权限 | 显示用户或角色的成员关系与权限。 | information_pane | M4 |
| F-INFO-009 | AI 助手入口 | 在信息窗格中与 AI 助手对话。 | information_pane, ai_chat | M11 |
| F-INFO-010 | 最大化与还原 | 右上角按钮一键最大化信息窗格宽度，再次点击还原。 | information_pane | M2 |

## DATA 数据编辑器（表查看器）

| 编号 | 功能 | 摘要与验收要点 | 帮助页 | 里程碑 |
| --- | --- | --- | --- | --- |
| F-DATA-001 | 网格视图 | 虚拟化结果网格，支持 10 万行流畅滚动；行号列、表头排序标记、NULL 显示样式与基线一致。 | data_rdbms, data_viewer | M2 |
| F-DATA-002 | 表单视图 | 单条记录以表单形式显示和编辑，可在网格与表单之间切换。 | data_edit | M2 |
| F-DATA-003 | 编辑记录 | 添加、编辑、删除记录；离开当前行或点击“应用”时提交；“放弃”还原；日期时间字段提供选择器。 | data_edit | M2 |
| F-DATA-004 | 事务 | 开始事务、提交、回滚；偏好设置中可设置自动开始事务。 | data_rdbms | M2 |
| F-DATA-005 | 区域选择与复制粘贴 | 单元格、行、列和矩形区域选择；复制为制表符文本、INSERT、UPDATE 语句；复制字段名；粘贴多行多列数据。 | data_edit | M2 |
| F-DATA-006 | 筛选与排序窗格 | 可视化条件构建器：字段、运算符（=、!=、<、<=、>、>=、包含、不包含、开头是、结尾是、是 null、是空的、介于、在列表等）、括号分组；排序多字段；切换为“文本”直接编辑 WHERE 子句。 | data_filter_wizard, data_filter | M2 |
| F-DATA-007 | 查找与替换 | 在结果中查找记录、查找列、替换；选项为递增搜索、大小写符合。 | data_sort_find | M2 |
| F-DATA-008 | 导航栏与分页 | 首页、上一页、页码、下一页、末页；每页记录数（默认 1000）；显示总行数；刷新；停止加载。 | data_navigation_bar | M2 |
| F-DATA-009 | 格式设置 | 设置列宽、行高，显示或隐藏列，显示或隐藏 ROWID [Lite]，固定列。 | data_format | M2 |
| F-DATA-010 | 原始数据模式 | 以原始格式显示和编辑数据（不做类型格式化）。 | data_raw | M2 |
| F-DATA-011 | 单元格编辑器 | 文本、十六进制、图像、网页、JSON、枚举与集合 [My] 等专用编辑器，显示在网格下方或信息窗格中。 | data_assistant_editors | M2 |
| F-DATA-012 | 外键数据选择 | 编辑外键字段时弹出被引用表的数据供选择。 | data_edit | M2 |
| F-DATA-013 | 字段信息 | 显示所选字段的数据类型与注释。 | data_field_comment | M2 |
| F-DATA-014 | 显示 SQL | 查看表查看器执行的 SELECT 语句和待提交的修改语句。 | data_rdbms | M2 |
| F-DATA-015 | 生成数据入口 | 工具栏“生成数据”跳转数据生成工具。 | data_rdbms | M9 |

## QRY 查询

| 编号 | 功能 | 摘要与验收要点 | 帮助页 | 里程碑 |
| --- | --- | --- | --- | --- |
| F-QRY-001 | 查询编辑器 | Tree-sitter SQL 语法高亮、括号高亮、代码折叠、行号、自动换行、缩放（⌘+、⌘-、⌘0）。 | query_editor | M3 |
| F-QRY-002 | 代码补全 | 基于代码补全信息（库、表、视图、字段、函数的缓存）补全关键字、对象和代码段；补全列表用编辑器自带的补全菜单。 | query_editor, preferences_code_completion | M3 |
| F-QRY-003 | 查找与替换 | 正则表达式、大小写符合、整个字。 | query_editor | M3 |
| F-QRY-004 | SQL 格式 | 缩进、注释与取消注释、转换大小写、美化 SQL（含选项：短括号长度、大写关键字）、简化 SQL。 | query_editor | M3 |
| F-QRY-005 | 剪贴板堆栈 | 保存最近复制的多段文本，⇧⌘V 选择粘贴。 | query_editor | M3 |
| F-QRY-006 | 运行 | 运行全部、运行已选择的、运行当前语句、停止；选择数据库或模式；运行多条语句。 | query_editor | M3 |
| F-QRY-007 | 查询结果 | 每条 SELECT 一个结果选项卡（最多 20 个，`-- NAME:` 命名）；消息和摘要页；固定选项卡；结果位置（下方、右侧、新页面）；结果网格复用 `navidog-grid`，来自单张有主键的表时可编辑；MySQL/MariaDB 可显示查询分析和状态。 | query_results | M3 |
| F-QRY-008 | 查询解释 | 执行计划的网格、文本、JSON 视图和可视化图；耗时阈值高亮；查找。 | query_explain | M3 |
| F-QRY-009 | 查询参数 | SQL 中的参数占位符在运行时弹出输入框。 | query_parameters | M3 |
| F-QRY-010 | 代码段 | 内置和自定义代码段，按数据库类型、标签过滤，带占位符。 | query_code_snippet | M3 |
| F-QRY-011 | 保存查询 | 查询保存在连接的设置位置下，出现在对象分类“查询”中；⌘S、⇧⌘S。 | query | M3 |
| F-QRY-012 | 历史日志 | ⇧⌘H 打开历史日志查看器，可显示日期、时间、服务器名、会话 ID、连接类型、运行时间。 | log_files | M3 |
| F-QRY-013 | 查询创建工具 | 可视化构建 SELECT：拖放表、选择输出字段、设置关系、筛选准则、分组、排序、子查询。与 ER 图共用画布，所以放在 M7。 | query_builder | M7 |
| F-QRY-014 | 命令列界面 | ⇧⌘C 打开交互式命令行（类似 `mysql`、`psql` 的终端会话）。 | console | M3 |
| F-QRY-015 | 运行 SQL 文件 | 对连接或数据库执行 SQL 文件，显示进度和错误，可选遇错继续。 | dump_execute_sql | M6 |
| F-QRY-016 | 转储 SQL 文件 | 导出数据库或表的结构和数据为 SQL 文件。 | dump_execute_sql | M6 |
| F-QRY-017 | 提取 SQL | 从备份文件中提取 SQL。 | extract_sql | M9 |
| F-QRY-018 | 询问 AI 与 AI 修复 | 在编辑器中选中 SQL 询问 AI、让 AI 修复报错的查询。 | query_ask_ai, query_fix_ai | M11 |
| F-QRY-019 | 调试器 [PG] | PL/pgSQL 函数调试：断点、单步、变量查看。 | debugger, debugger_postgresql | M14 |

## DSG 对象设计器

| 编号 | 功能 | 摘要与验收要点 | 帮助页 | 里程碑 |
| --- | --- | --- | --- | --- |
| F-DSG-001 | 表设计器：字段 | 字段网格（名称、类型、长度、小数点、不是 null、虚拟、键、注释），下方字段属性区按类型变化；⌘+ 添加、⌘- 删除、⌘D 复制字段、⇧⌘K 设为主键、⌘↑ / ⌘↓ 移动。 | mysql_table, postgresql_table, sqlite_table, hot_keys | M4 |
| F-DSG-002 | 表设计器：索引 | 名称、字段、索引类型、索引方法、注释；PostgreSQL 支持部分索引与表达式索引。 | mysql_table, postgresql_table, sqlite_table | M4 |
| F-DSG-003 | 表设计器：外键 | 名称、字段、被引用模式/表/字段、删除时、更新时。 | 同上 | M4 |
| F-DSG-004 | 表设计器：唯一与检查 | 唯一约束 [PG][Lite]，检查约束（MySQL 8.0.16+、PostgreSQL、SQLite）。 | 同上 | M4 |
| F-DSG-005 | 表设计器：排除约束 [PG] | 排除约束。 | postgresql_table | M4 |
| F-DSG-006 | 表设计器：触发器 | 名称、触发时机、事件、定义。 | 同上 | M4 |
| F-DSG-007 | 表设计器：选项 | MySQL：引擎、字符集、排序规则、自动递增、行格式等；PostgreSQL：拥有者、表空间、继承、填充因子等；SQLite：WITHOUT ROWID、STRICT。 | 同上 | M4 |
| F-DSG-008 | 表设计器：分区 [PG][My] | 分区方式和分区列表。 | postgresql_table, mysql_table | M4 |
| F-DSG-009 | 表设计器：注释与 SQL 预览 | 表注释；SQL 预览页显示将执行的 CREATE 或 ALTER 语句。 | 同上 | M4 |
| F-DSG-010 | 表设计器：保存 | 新表生成 CREATE，已有表按差异生成 ALTER（SQLite 需要重建表时自动生成迁移语句），执行失败时保留编辑内容。 | 同上 | M4 |
| F-DSG-011 | 视图设计器 | 定义、高级选项（算法、安全性、检查选项）、SQL 预览；视图创建工具（复用查询创建工具）；预览数据。 | mysql_view, postgresql_view, sqlite_view | M4 |
| F-DSG-012 | 实体化视图 [PG] | 设计、刷新实体化视图。 | postgresql_mview | M4 |
| F-DSG-013 | 函数与过程设计器 | 新建向导（参数、返回类型）、定义编辑器、运行并输入参数、查看结果。 | mysql_function, postgresql_function | M4 |
| F-DSG-014 | 事件设计器 [My] | 定义、计划（一次、每隔）、状态。 | mysql_event | M4 |
| F-DSG-015 | 数据库与模式 | 新建、编辑、删除数据库（字符集、排序规则）；PostgreSQL 模式的新建、编辑、删除。 | mysql_database, postgresql_database, sqlite_database | M4 |
| F-DSG-016 | 附加数据库 [Lite] | 附加、分离 SQLite 数据库文件。 | sqlite_database | M4 |
| F-DSG-017 | 表空间 | MySQL 表空间、PostgreSQL 表空间的设计。 | mysql_tablespace | M4 |
| F-DSG-018 | PostgreSQL 其他对象 [PG] | 类型、域、序列、扩展管理、外部服务器、聚合、转换、运算符、排序规则等。 | postgresql_type, postgresql_extension, postgresql_foreign_server, postgresql_others | M4 |
| F-DSG-019 | MySQL 其他对象 [My] | 其他对象分类下的对象设计。 | mysql_others | M4 |
| F-DSG-020 | SQLite 其他对象 [Lite] | 索引、触发器等独立对象。 | sqlite_others | M4 |

## SEC 用户与安全

| 编号 | 功能 | 摘要与验收要点 | 帮助页 | 里程碑 |
| --- | --- | --- | --- | --- |
| F-SEC-001 | MySQL 用户设计器 [My] | 常规（用户名、主机、插件、密码）、高级（资源限制、SSL）、成员属于、服务器权限、对象权限、SQL 预览。 | security_mysql_user | M4 |
| F-SEC-002 | MySQL 角色设计器 [My] | MySQL 8 角色的成员与权限。 | security_mysql_role | M4 |
| F-SEC-003 | PostgreSQL 用户、角色、组设计器 [PG] | 常规属性、成员、成员属于、权限、SQL 预览。 | security_postgresql_user, security_postgresql_role, security_postgresql_group | M4 |
| F-SEC-004 | 权限管理员 | 以对象为中心批量授予或撤销权限。 | security_privilege_manager | M4 |
| F-SEC-005 | SQLite 用户管理 [Lite] | 针对启用用户认证扩展的 SQLite 管理用户。 | security_sqlite | M4 |

## IO 导入导出

| 编号 | 功能 | 摘要与验收要点 | 帮助页 | 里程碑 |
| --- | --- | --- | --- | --- |
| F-IO-001 | 导入向导 | 八步：选择格式（TXT、CSV、Excel、XML、JSON 等）、选择文件、记录格式、格式选项、映射表、映射字段、导入模式（追加、更新、追加或更新、删除、复制）、保存并开始；可保存为配置文件。 | import_wizard, import_wizard_1 ~ import_wizard_8 | M6 |
| F-IO-002 | 导出向导 | 五步：选择格式（TXT、CSV、Excel、HTML、XML、JSON、SQL、Markdown 等）、保存路径、选择字段、附加选项、保存并开始。 | export_wizard, export_wizard_1 ~ export_wizard_5 | M6 |
| F-IO-003 | 导出结果集 | 从结果网格导出当前结果或选中区域。 | export_wizard | M6 |
| F-IO-004 | 插件格式 | 导入导出格式注册表，插件可以新增格式。 | — | M6（注册表）、M5（JS） |

## XFER 数据传输与同步

| 编号 | 功能 | 摘要与验收要点 | 帮助页 | 里程碑 |
| --- | --- | --- | --- | --- |
| F-XFER-001 | 数据传输 | 三步：选择源和目标连接（或文件）与高级选项、选择对象和传输模式、确认并开始；跨数据库类型传输时做类型映射；⇧⌘T。 | data_transfer, data_transfer_1 ~ data_transfer_3 | M8 |
| F-XFER-002 | 数据同步 | 四步：选择连接和比较选项、表映射、查看数据比较结果（新增、修改、删除行）、编辑并运行脚本。 | data_sync, data_sync_1 ~ data_sync_4 | M8 |
| F-XFER-003 | 结构同步 | 三步：选择连接和比较选项、查看结构比较结果（对象树加 DDL 对比）、编辑并运行脚本。 | structure_sync, structure_sync_1 ~ structure_sync_3 | M8 |
| F-XFER-004 | 数据迁移工具 | 面向异构库的迁移流程。 | data_migration | M8 |

## MDL 模型

| 编号 | 功能 | 摘要与验收要点 | 帮助页 | 里程碑 |
| --- | --- | --- | --- | --- |
| F-MDL-001 | 模型工作区 | 模型列表、新建模型（概念、逻辑、物理），工作区布局。 | model, model_workspace | M7 |
| F-MDL-002 | 图表画布 | 自绘画布：表、视图、外键连线、笔记、标签、图像、形状、图层；缩放、平移、对齐网格、自动布局。 | model_diagram, model_diagram_canvas, model_diagram_* | M7 |
| F-MDL-003 | 概念与逻辑模型 | 实体、关系，模型之间转换。 | model_model_conceptual, model_model_logical, model_conversion | M7 |
| F-MDL-004 | 物理模型 | 关系型、维度（事实、维度、桥接、外展）、数据仓库（中心、链接、附属、时间点、参考）。 | model_model_relational, model_model_dimensional, model_model_data_vault | M7 |
| F-MDL-005 | 逆向工程 | 把现有数据库逆向生成模型。 | model_reverse_eng, model_reverse_to_model | M7 |
| F-MDL-006 | 正向工程 | 同步模型到数据库（比较并生成脚本）、导出模型为 SQL。 | model_forward_eng, model_sync_to_database, model_export_sql | M7 |
| F-MDL-007 | 同步数据库到模型 | 把数据库的变更同步回模型。 | model_sync_to_model | M7 |
| F-MDL-008 | 比较模型 | 比较两个模型工作区。 | model_compare | M7 |
| F-MDL-009 | 打印与导出图表 | 导出 PNG、SVG、PDF，打印预览。 | model_diagram_export, model_diagram_preview | M7 |
| F-MDL-010 | 模型数据字典 | 由模型生成数据字典文档。 | model_data_dictionary, model_data_dictionary_create | M7 |

## AUTO 备份与自动运行

| 编号 | 功能 | 摘要与验收要点 | 帮助页 | 里程碑 |
| --- | --- | --- | --- | --- |
| F-AUTO-001 | 内置备份 | 备份数据库或模式的对象与数据为 navidog 备份文件，可选对象、压缩、加密；备份列表出现在对象分类“备份”中。 | backup, backup_restore, build_in_backup_restore | M9 |
| F-AUTO-002 | 还原 | 从备份文件还原，选择对象与选项，显示进度和日志。 | restore | M9 |
| F-AUTO-003 | 备份配置文件 | 保存备份设置以复用。 | backup | M9 |
| F-AUTO-004 | 自动运行：批处理作业 | 第一步创建作业：添加查询、备份、导入导出、数据传输、数据同步等任务；设置通知邮件。 | automation, automation_task | M9 |
| F-AUTO-005 | 自动运行：计划 | 第二步设置计划；macOS 上用 launchd 用户代理执行，navidog 未运行时也能执行。 | automation_schedule | M9 |

## TOOL 工具

| 编号 | 功能 | 摘要与验收要点 | 帮助页 | 里程碑 |
| --- | --- | --- | --- | --- |
| F-TOOL-001 | 服务器监控 | 进程列表（可结束进程）、变量（可编辑）、状态，自动刷新。 | server_monitor | M4 |
| F-TOOL-002 | 在数据库或模式中查找 | ⇧⌘F 在对象名、数据中查找字符串。 | find_in_db | M4 |
| F-TOOL-003 | 模式分析 | 分析模式的统计信息与问题。 | schema_analysis | M8 |
| F-TOOL-004 | 数据字典 | 五步向导生成数据库文档（选择数据库、对象和顺序、模板、布局、输出路径）。 | data_dictionary, data_dictionary_1 ~ data_dictionary_5 | M8 |
| F-TOOL-005 | 数据分析 | 对表或结果集做数据剖析：分布、空值、唯一值、统计图。 | data_profiling, data_profiling_1, data_profiling_2 | M10 |
| F-TOOL-006 | 数据生成 | 三步向导，按列选择生成器（姓名、地址、邮箱、电话、日期、数字、序列、正则、外键、UUID 等约 40 种）并预览。 | data_generate, data_generate_* | M9 |

## BI

| 编号 | 功能 | 摘要与验收要点 | 帮助页 | 里程碑 |
| --- | --- | --- | --- | --- |
| F-BI-001 | BI 工作区 | 数据源、图表、仪表板三类对象的工作区。 | bi, bi_workspace | M10 |
| F-BI-002 | 数据源 | 由连接、查询或文件创建数据源；自定义字段、筛选排序、修改连接、数据分析。 | bi_datasource, bi_datasource_* | M10 |
| F-BI-003 | 图表 | 条形图、折线或面积图、条形与折线组合、饼图或圆环图、散点图、热图、仪表盘、KPI、数据透视表、表、树状图、瀑布图、飓风图、趋势、值、控件；聚合、日期时间格式、范围、筛选排序。 | bi_chart, bi_chart_* | M10 |
| F-BI-004 | 仪表板 | 添加图表、图像、文本标签、形状、页面；排列；演示模式；打印与导出。 | bi_dashboard, bi_dashboard_* | M10 |
| F-BI-005 | 布景主题 | 切换图表与仪表板的配色主题。 | bi_theme | M10 |

## AI 助手

| 编号 | 功能 | 摘要与验收要点 | 帮助页 | 里程碑 |
| --- | --- | --- | --- | --- |
| F-AI-001 | 配置 | 接入 OpenAI 兼容接口：Base URL、API Key（加密存储）、模型选择。 | ai_create, preferences_ai | M11 |
| F-AI-002 | 与 AI 聊天 | 在信息窗格中对话；可附加 schema（选择对象）作为上下文；回答中的 SQL 一键插入编辑器。 | ai, ai_chat | M11 |
| F-AI-003 | 询问 AI 与 AI 修复查询 | 见 F-QRY-018。 | query_ask_ai, query_fix_ai | M11 |

## SYNC 同步（替代 Navicat Cloud）

| 编号 | 功能 | 摘要与验收要点 | 帮助页 | 里程碑 |
| --- | --- | --- | --- | --- |
| F-SYNC-001 | 同步目录 | 把连接（不含明文密码）、查询、代码段、模型、虚拟组同步到用户指定的本地目录（可放在 iCloud Drive、Dropbox 等）。 | collaboration | M12 |
| F-SYNC-002 | Git 同步 | 同步目录可作为 Git 仓库，提供提交、拉取、推送和冲突提示。 | collaboration_push | M12 |
| F-SYNC-003 | 冲突解决 | 同一对象两端都修改时，显示差异并选择保留哪一份。 | — | M12 |
| F-SYNC-004 | 导航树显示 | 同步目录在导航树中与“我的连接”并列显示。 | navigation_pane | M12 |

## PLUG 插件

| 编号 | 功能 | 摘要与验收要点 | 帮助页 | 里程碑 |
| --- | --- | --- | --- | --- |
| F-PLUG-001 | 插件加载 | 从插件目录发现 `gpui-shell.json`，校验 manifest，列出插件。 | — | M5 |
| F-PLUG-002 | 能力授权 | 首次启用时弹窗列出 manifest 申请的能力，用户逐项确认；授权结果持久化，可撤销。 | — | M5 |
| F-PLUG-003 | 命令与菜单 | 插件注册命令，出现在菜单、右键菜单或工具栏中。 | — | M5 |
| F-PLUG-004 | 导入导出格式 | 插件注册新的导入导出格式。 | — | M5 |
| F-PLUG-005 | 单元格格式化器 | 插件注册声明式格式化规则，用于结果网格显示。 | — | M5 |
| F-PLUG-006 | JS 驱动 | 插件实现新的数据库驱动，通过 `net` 或 HTTP 能力访问数据库。 | — | M5 |
| F-PLUG-007 | Dock 面板 | 插件注册面板，作为 Dock Panel 显示。 | — | M5 |
| F-PLUG-008 | 插件管理 | 偏好设置中的插件页：启用、停用、查看授权、重新加载。 | — | M5 |

## REL 发布

| 编号 | 功能 | 摘要与验收要点 | 帮助页 | 里程碑 |
| --- | --- | --- | --- | --- |
| F-REL-001 | 打包 | macOS `.app` 与 DMG，Windows zip、MSI 与 `.msi.zip`，Linux `.deb`、`.rpm`、AppImage 与 tarball，均为 x86_64 和 aarch64。tag 发布附带 `latest.json` 和 `SHA256SUMS`。 | — | M15 |
| F-REL-002 | 签名与公证 | macOS：完整应用打包后，无证书走免费 ad-hoc 签名并严格验签；有证书保留 hardened runtime 与正式签名，配齐凭据后公证。免费包仍可能需要在系统设置中允许打开；见 150 规格。 | — | M15 |
| F-REL-003 | 自动更新 | 两个渠道都读同一份 `latest.json` 里的 `releases`；不含预发布时跳过预发布和 nightly。下载后按 SHA-256 校验。macOS 能写到当前 `.app` 时就地替换，否则交给系统打开安装包。 | — | M15 |
| F-REL-004 | 迁移或升级 | 配置目录版本化，升级时自动迁移旧配置。 | migration | M15 |
