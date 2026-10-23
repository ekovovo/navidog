# M4 作者 review 修复

范围：[PR #3](https://github.com/xhofe/navidog/pull/3)，作者在 `1c26253` 的 review；本记录只跟踪该次意见。规格补充独立提交 `c5b7a77`。

## 修复清单

| 编号 | 问题 | 状态 | 验证 |
| --- | --- | --- | --- |
| 01 | 设计器缺少 ObjectRef，收藏/启动项/断线关闭失效 | 已修 | tabs：启动项、断线关闭；旧布局从 baseline 恢复引用 |
| 02 | 重复设计同表新建多个草稿 | 已修 | tabs：设计/数据分别复用，弹出窗口复用 |
| 03 | 保存/重载后表单与条目顺序不同，误写另一条目 | 已修 | designer：重排快照后真实输入，只改变当前约束 |
| 04 | 跨 crate 翻译键显示原 key | 已修 | key 移入 app.yml，含 en/zh-CN |
| 05 | MySQL/MariaDB 独立 DROP INDEX 缺表名 | 已修 | 计划单测及 MySQL/MariaDB 实库删除 |
| 06 | 字段选择未归一化已有引用形式 | 已修 | designer：未引用字段取消选择、重新添加 |
| 07 | 重载成功后残留 invalid_inputs | 已修 | designer：重载清理错误并允许后续编辑 |
| 08 | POINT/INTERVAL 被当成数值类型 | 已修 | 改用 util 的 type_kind；新增分类断言 |
| 09 | 界面校验写死英文 | 已修 | 注释 NUL、拥有者和非关系连接均使用双语 key |
| 10 | 未编辑新表与首次加载失败误报修改 | 已修 | designer：新表不脏；tabs：首次失败直接关闭；保存后 clean 的实库 UI 回归 |
| 11 | SQL 预览删除按钮可用 | 已修 | 选择状态按页类型判断；浅深色预览截图 |
| 12 | 截图数量与实库前提不清 | 已修 | README 明确 44/60 全应用、17/33 设计器，历史验收注明批次 |
| 13 | 表菜单与 ⌘D 绑定旧空实现 | 已修 | core 共用 DesignTable；tabs：查看器真实快捷键打开设计器 |
| 14 | 输入实体数量写死 | 已修 | 使用 PROPERTY_IDS/ENTRY_KEYS 长度 |
| 15 | 引用模式/表/字段的位置数字重复 | 已修 | 使用具名常量；级联 UI 回归通过 |

## 验证

- 格式、裸值检查、clippy、diff 空白检查通过。
- `NAVIDOG_LIVE=1 cargo test --workspace --locked`：226 项，35 套件通过；另复验 tabs 新增快捷键与弹出窗口断言。
- deny 四项通过，audit 通过，保留 9 条项目已允许警告。
- `NAVIDOG_LIVE=1 cargo test -p navidog --test snapshot --locked`：生成 60 张，逐张检查；33 张设计器原图同步归档，其中 16 张内容更新。
- 性能和验收边界见 [M4 验收记录](../pr/acceptance-m4.md)。Navicat 运行态并排验收仍未完成。

跨平台补充：查看器设计命令使用 `secondary-d`（macOS 为 ⌘D，其他平台为 Ctrl+D），与 headless 快捷键测试一致。首次远程 Integration 在启动数据库阶段遇到 Docker Hub `toomanyrequests`，尚未执行代码测试；最终提交的运行结果单独查看。
