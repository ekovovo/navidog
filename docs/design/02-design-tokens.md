# 设计 token 表

`navidog-ui` 中的每个 token 都必须在本表登记，并写明来源。界面代码只引用 token 名，不写数值（规则见 [00-overview.md](00-overview.md#5-gpui-kit-规范在本项目中的落地)）。

## 1. 单位与换算

- 表中尺寸单位一律为 macOS 的 pt，与 nib 的 `NSFrame` 和 AX 的测量值相同，1pt 在 Retina 屏上对应 2 个物理像素。
- rem 基准是 `theme.font_size = 13`。token 定义处用 `pt(n)` 把 pt 换算成 `rems(n / 13)`，因此应用缩放时所有 token 一起等比变化。
- nib 坐标原点在左下角；表中的边距和偏移已经换算成以左上角为原点。

**来源写法**
- `nib:<文件>/<控件路径>`：取自 `cargo xtask nib-tree <文件>` 的输出。
- `ax:<窗口>/<元素>`：取自 `tools/ax-dump.swift` 的输出。
- `appkit:<颜色名>`：取自 `tools/colors-dump.swift` 的输出。
- `暂定`：还没有实测来源的估计值，必须在对应里程碑验收前用 AX 实测替换。

## 2. 字体

| token | 值 | 来源 |
| --- | --- | --- |
| `font.family` | `.SystemUIFont`（SF Pro） | nib：所有控件都用 `.AppleSystemUIFont` |
| `font.mono_family` | `Menlo` | 暂定，M3 查询编辑器规格中确认 |
| `font.size.body` | 13 | nib:ConnectionFormGeneralModule_mysql/NSTextField（13.0） |
| `font.size.small` | 11 | nib:ConnectionFormGeneralModule_mysql/备注文字（11.0）；nib:MainWindowController/状态栏文字（高 14） |
| `font.weight.heading` | Semibold | nib:ConnectionFormConnTypeSelectionModule/标题（`.AppleSystemUIFontDemi` 13） |
| `font.line.body` | 16 | nib：13pt 标签高 16 |
| `font.line.small` | 14 | nib：11pt 文字高 14 |

## 3. 主窗口

| token | 值 | 来源 |
| --- | --- | --- |
| `window.default_size` | 1240 × 800（内容区） | nib:MainWindowController/NSWindowTemplate.NSWindowRect |
| `window.toolbar_style` | Expanded（大图标加文字，位于标题下方） | nib:MainWindowController/NSWindowToolbarStyle = 1 |
| `toolbar.height` | 52 | 暂定：Expanded 样式工具栏的典型高度，待 `ax:主窗口/AXToolbar` |
| `toolbar.item_icon` | 24 | 暂定，待 AX |
| `toolbar.item_min_width` | 52 | 暂定，待 AX |
| `toolbar.item_gap` | 8 | 暂定，待 AX |
| `pane.navigator.width` | 240 | nib:MainWindowController/NSSplitView/第 1 栏 |
| `pane.object.width` | 734 | nib:MainWindowController/NSSplitView/第 2 栏 |
| `pane.info.width` | 264 | nib:MainWindowController/NSSplitView/第 3 栏 |
| `pane.divider` | 1 | nib:MainWindowController/NSSplitView（栏间距 1） |
| `statusbar.height` | 24 | nib:MainWindowController/NSView（split 从 y=24 开始） |
| `statusbar.text_inset_x` | 18 | nib:MainWindowController/NSTextField（x=18，宽 1204 = 1240 − 2×18） |
| `statusbar.text_offset_y` | 5 | nib:MainWindowController/NSTextField（y=5，高 14） |

## 4. 导航窗格

| token | 值 | 来源 |
| --- | --- | --- |
| `navigator.filter.height` | 34 | nib:NAVConnectionTreeViewController/NSStackView（高 34） |
| `navigator.filter.inset_x` | 10 | nib:NAVConnectionTreeViewController/NSStackView（x=10） |
| `navigator.filter.search_height` | 24 | nib:NAVConnectionTreeViewController/NSSearchField（高 24，y=5） |
| `navigator.filter.icon` | 16 | nib:NAVConnectionTreeViewController/筛选图标（16×16） |
| `navigator.filter.icon_gap` | 8 | nib:NAVConnectionTreeViewController/筛选图标间距（x=0、24、48） |
| `navigator.filter.search_icon_gap` | 8 | nib:NAVConnectionTreeViewController（搜索框宽 490，图标组 x=498） |
| `navigator.row.height` | 22 | 暂定，待 `ax:主窗口/AXOutline/AXRow` |
| `navigator.row.indent` | 16 | 暂定，待 AX |
| `navigator.row.icon` | 16 | 暂定，待 AX |

## 5. 对象窗格与选项卡

| token | 值 | 来源 |
| --- | --- | --- |
| `tabbar.height` | 37 | nib:NAVContentTabBarViewController/NSStackView（高 37） |
| `tabbar.separator` | 1 | nib:NAVContentTabBarViewController/NSBox（分隔线，框高 5，线宽 1） |
| `tab.min_width` | 120 | 暂定，待 AX |
| `object_toolbar.height` | 36 | 暂定，待 AX |
| `list.header.height` | 23 | nib:MainObjectViewController/NSTableHeaderView（高 23） |
| `list.row.height` | 22 | 暂定，待 AX |
| `list.name_column_width` | 220 | 暂定，待 AX（详细信息视图的名称列） |
| `list.detail_column_width` | 130 | 暂定，待 AX（详细信息视图的属性列） |

## 5.1 表查看器与结果网格（M2）

| token | 值 | 来源 |
| --- | --- | --- |
| `grid.header.height` | 28 | nib:GridViewController/NSTableHeaderView（高 28） |
| `grid.type_header.height` | 34 | 暂定。显示类型时表头单独用这个高度（`DataTable::header_row_height`）；28 放不下字段名加类型。数据行仍用 `grid.row.height` |
| `grid.row_number_width` | 20 | nib:GridViewController/行号 NSScrollView（宽 20） |
| `grid.row.height` | 20 | 暂定，待 AX |
| `grid.date_picker.width` | 317 | nib:NAVDateTimePicker/NSWindowRect（317 × 247） |
| `viewer.bottom_bar.height` | 28 | nib:TableViewViewController/底部栏（1153 × 28） |
| `viewer.bottom_bar.button` | 20 × 16 | nib:TableViewViewController/NSButton |
| `viewer.limit.width` | 212 | nib:TableViewViewController/“Limit … records per page”（212 × 24，11pt） |
| `viewer.pager.width` | 86 | nib:TableViewViewController/分页按钮组（86 × 27，按钮间距 28） |
| `filter.segmented.width` | 115 | nib:FilterAndSortManager/NSSegmentedControl（115 × 24） |
| `filter.builder.max_height` | 160 | 暂定，待 AX。条件多时仍给网格留出空间 |

## 6. 信息窗格

| token | 值 | 来源 |
| --- | --- | --- |
| `info.header.height` | 30 | nib:InformationViewViewController/NSClassSwapper（400 × 30） |
| `info.header.button` | 16 | nib:InformationViewViewController/NSButton（16×16，距右 8，y=7） |
| `info.header.button_inset` | 8 | 同上 |

## 7. 表单与对话框

| token | 值 | 来源 |
| --- | --- | --- |
| `form.margin` | 20 | nib:ConnectionFormGeneralModule_mysql/NSStackView（x=20） |
| `form.row.height` | 24 | nib:ConnectionFormGeneralModule_mysql/行（高 24） |
| `form.row.gap` | 8 | nib:ConnectionFormGeneralModule_mysql/行（相邻行 y 相差 32） |
| `form.row.compact_height` | 22 | nib:ConnectionFormAdvancedModule_mysql/行（高 22） |
| `form.row.compact_gap` | 8 | nib:ConnectionFormAdvancedModule_mysql/行（相邻行 y 相差 30） |
| `form.label.height` | 16 | nib:ConnectionFormGeneralModule_mysql/标签（高 16，y=4） |
| `form.label_gap` | 6 | nib:ConnectionFormGeneralModule_mysql（标签右边缘 610，输入框 x=616） |
| `form.field.width` | 300 | nib:ConnectionFormGeneralModule_mysql/NSTextField（宽 300） |
| `form.field.width_short` | 80 | nib:ConnectionFormGeneralModule_mysql/端口（宽 80） |
| `form.field.width_wide` | 692 | nib:ConnectionFormSSLModule/证书路径（宽 692） |
| `form.popup.height` | 20 | nib:ConnectionFormGeneralModule_mysql/NSPopUpButton（高 20，y=2） |
| `form.popup.width` | 300 | 同上 |
| `form.popup.width_medium` | 280 | nib:ConnectionFormAdvancedModule_mysql/NSPopUpButton（宽 280） |
| `form.popup.width_short` | 196 | nib:ConnectionFormSSHModule/验证方法（宽 196） |
| `form.checkbox.height` | 16 | nib:ConnectionFormAdvancedModule_mysql/NSButton（高 16） |
| `form.note.height` | 14 | nib:ConnectionFormGeneralModule_mysql/备注（11pt，高 14） |
| `form.list.header_height` | 23 | nib:ConnectionFormCustomDbListModule/NSTableHeaderView（高 23） |
| `form.list.row_height` | 17 | nib:ConnectionFormCustomDbListModule/NSTableCellView（高 17） |
| `form.list.button` | 24 × 22 | nib:ConnectionFormCustomDbListModule/加减按钮 |
| `button.height` | 24 | nib:ConnectionFormPopoverWindowController/“Copy to Clipboard”（134 × 24，含边框）；macOS 26 控件尺寸可能变化，待 AX |
| `conn_type.window` | 925 × 365 | nib:ConnectionFormConnTypeSelectionModule/NSSplitView |
| `conn_type.sidebar_width` | 317 | 同上，第 1 栏 |
| `conn_type.header_height` | 40 | nib:ConnectionFormConnTypeSelectionModule/标题行（高 40） |
| `conn_type.search_row_height` | 36 | nib:ConnectionFormConnTypeSelectionModule/搜索行（高 36） |
| `conn_type.search_width` | 200 | nib:ConnectionFormConnTypeSelectionModule/NSSearchField（200 × 22） |
| `conn_type.footer_height` | 45 | nib:ConnectionFormConnTypeSelectionModule/底部栏（高 45） |

## 7.1 偏好设置

| token | 值 | 来源 |
| --- | --- | --- |
| `preferences.width` | 640 | nib:PreferencesWindowController/NSWindowRect |
| `preferences.height` | 480 | 同上 |
| `preferences.nav_width` | 160 | 暂定，待 AX（左侧分类列表；nib 用自动布局，没有帧） |

## 8. 颜色

Navicat 的 nib 中几乎所有颜色都是 AppKit 语义色。在 718 个 nib 中，`textBackgroundColor`、`controlTextColor`、`controlColor`、`labelColor` 各出现两千多次，`gridColor`、`separatorColor` 出现数百次。所以 navidog 的颜色 token 直接对应语义色，取值为本机 macOS 26.6 上的实测结果，格式为 `#RRGGBBAA`。

| token | AppKit 来源 | 浅色 | 深色 |
| --- | --- | --- | --- |
| `color.window_bg` | appkit:windowBackgroundColor | `#ffffffff` | `#1e1e1eff` |
| `color.control_bg` | appkit:controlBackgroundColor | `#ffffffff` | `#1e1e1eff` |
| `color.text_bg` | appkit:textBackgroundColor | `#ffffffff` | `#1e1e1eff` |
| `color.under_page_bg` | appkit:underPageBackgroundColor | `#969696e5` | `#282828ff` |
| `color.control` | appkit:controlColor | `#ffffffff` | `#ffffff3f` |
| `color.label` | appkit:labelColor | `#000000d8` | `#ffffffd8` |
| `color.label_secondary` | appkit:secondaryLabelColor | `#0000007f` | `#ffffff8c` |
| `color.label_tertiary` | appkit:tertiaryLabelColor | `#00000042` | `#ffffff3f` |
| `color.label_quaternary` | appkit:quaternaryLabelColor | `#00000019` | `#ffffff19` |
| `color.text` | appkit:textColor | `#000000ff` | `#ffffffff` |
| `color.control_text` | appkit:controlTextColor | `#000000d8` | `#ffffffd8` |
| `color.disabled_text` | appkit:disabledControlTextColor | `#0000003f` | `#ffffff3f` |
| `color.placeholder` | appkit:placeholderTextColor | `#0000007f` | `#ffffff8c` |
| `color.header_text` | appkit:headerTextColor | `#000000d8` | `#ffffffff` |
| `color.separator` | appkit:separatorColor | `#00000019` | `#ffffff19` |
| `color.grid` | appkit:gridColor | `#e6e6e6ff` | `#1a1a1aff` |
| `color.row_alt` | appkit:alternatingContentBackgroundColors[1] | `#f4f5f5ff` | `#ffffff0c` |
| `color.accent` | appkit:controlAccentColor | `#007affff` | `#007affff` |
| `color.selection` | appkit:selectedContentBackgroundColor | `#0064e1ff` | `#0059d1ff` |
| `color.selection_inactive` | appkit:unemphasizedSelectedContentBackgroundColor | `#dcdcdcff` | `#464646ff` |
| `color.selection_text` | appkit:alternateSelectedControlTextColor | `#ffffffff` | `#ffffffff` |
| `color.text_selection` | appkit:selectedTextBackgroundColor | `#b3d7ffff` | `#3f638bff` |
| `color.focus_ring` | appkit:keyboardFocusIndicatorColor | `#0067f47f` | `#1aa9ff7f` |
| `color.link` | appkit:linkColor | `#0068daff` | `#419cffff` |
| `color.find_highlight` | appkit:findHighlightColor | `#ffff00ff` | `#ffff00ff` |
| `color.fill` | appkit:systemFill | `#00000019` | `#ffffff19` |
| `color.fill_secondary` | appkit:secondarySystemFill | `#00000014` | `#ffffff14` |
| `color.fill_tertiary` | appkit:tertiarySystemFill | `#0000000c` | `#ffffff0c` |
| `color.danger` | appkit:systemRed | `#ff383cff` | `#ff4245ff` |
| `color.warning` | appkit:systemOrange | `#ff8d28ff` | `#ff9230ff` |
| `color.success` | appkit:systemGreen | `#34c759ff` | `#30d158ff` |
| `color.info` | appkit:systemBlue | `#0088ffff` | `#0091ffff` |
| `color.gray` | appkit:systemGray | `#8e8e93ff` | `#98989dff` |

结果网格的修改标记不是 AppKit 语义色，先用暂定值，待与 Navicat 并排实测后替换：

| token | 来源 | 浅色 | 深色 |
| --- | --- | --- | --- |
| `color.grid_edited` | 暂定，待实测（已修改、未提交的单元格） | `#fff3c4ff` | `#5c4b1aff` |
| `color.grid_deleted` | 暂定，待实测（待删除的行） | `#ffd9d9ff` | `#5c2424ff` |
| `color.grid_inserted` | 深色截图里的新增行，暂定 | `#f0ddf6ff` | `#4a2960ff` |
| `color.grid_line` | 深色截图里的列分割线，暂定；`themes/navicat.json` 的 `table.row.border` 用同一色 | `#c8c8c8ff` | `#3a3a3aff` |
| `table.hover.background` | 暂定。gpui-kit 默认浅色是 `#f5f5f5`，盖在它自己的隔行色 `#fafafa` 上。这里的隔行色已是 `#f4f5f5` / `#ffffff0c`，悬停色要比奇数行和偶数行都更深（深色则更亮），否则偶数行悬停后和奇数行一样 | `#e6e6e8ff` | `#333333ff` |
| `color.chrome_active` | 深色截图里选中的选项卡和表查看器工具栏按钮，暂定；`tab.active.background` 用同一色。深色 `table.head.background` 为 `#2b2b2bff`，浅色为 `#f6f6f6ff` | `#d5e0eeff` | `#546886ff` |
| `color.icon_blue` | 工具栏和导航图标，暂定 | `#007affff` | `#5eb0f6ff` |
| `color.icon_green` | 同上 | `#248a3dff` | `#62c07aff` |
| `color.icon_orange` | 同上 | `#e07a00ff` | `#f0a04bff` |
| `color.icon_purple` | 同上 | `#8944abff` | `#c792eaff` |
| `color.icon_yellow` | 同上 | `#c4920aff` | `#e6c15aff` |
| `color.icon_teal` | 同上 | `#0e8a80ff` | `#4ecdc4ff` |
| `color.find_highlight` | `findHighlightColor`（即 systemYellow，管理连接窗口等处的搜索高亮，文字固定用黑色） | `#ffcc00ff` | `#ffd60aff` |

SQL 编辑器（DDL 页、DDL 选项卡，以及 M3 的查询编辑器）的底色、文字、当前行、行号分别取 `color.control_bg`、`color.label`、`color.fill_tertiary`、`color.label_tertiary`，写在 `themes/navicat.json` 的 `highlight` 中；语法配色暂用 gpui-component 默认主题的值，待与 Navicat 并排实测后替换。

连接颜色（F-CONN-014）的候选色板使用 systemRed、systemOrange、systemYellow、systemGreen、systemBlue、systemPurple、systemGray，取值见 `reference/colors.json`，具体列表在 M1 连接规格中确认。

## 9. 待 AX 实测的项目

以下 token 目前是估计值，对应里程碑验收前必须替换成 AX 实测值。运行方法：给终端授予辅助功能权限，打开 Navicat 并把主窗口调整到 1240 × 800，然后执行 `swift tools/ax-dump.swift`。

- `toolbar.*`：主工具栏高度、图标尺寸、按钮最小宽度和间距（M1）。
- `navigator.row.*`：导航树行高、缩进、图标尺寸（M1）。
- `tab.min_width`、`object_toolbar.height`、`list.row.height`（M2）。
- `button.height`：macOS 26 上按钮的实际高度（M1）。
- 连接表单窗口的整体尺寸和标签页切换控件（M1）：这部分由代码动态生成，nib 里没有。
