# 审查本分支

完成标准：每个严重、重要结论都回到代码里核实过；CR 文件写好并提交；回复里给出结论和问题清单。

## 1. 定范围

```bash
git branch --show-current
git merge-base HEAD master && git log --oneline master..HEAD && git diff --stat master...HEAD
```

分支不是从 `master` 拉出来的（例如从里程碑分支），就对几个候选分支各算 `git merge-base` 和 `git rev-list --count <base>..HEAD`，取最近的那个。

读 progress.md 第 2~4 节（分支声称完成了什么、已登记的偏差）、diff 涉及的模块规格，以及 `git diff master...HEAD -- docs/ AGENTS.md`，弄清分支自己声称做了什么。已登记的偏差不算问题，除非实现和登记的不符。

## 2. 后台跑检查

把 SKILL.md 的检查命令拼成一条，用 `block_until_ms: 0` 放到后台，输出写进 `/tmp` 下的日志，接着往下审。测试失败时按第 5 步二分。

## 3. 按模块派子代理

按模块给 diff 里的文件分组，每组大约 1500~4000 行改动（例如：表格模型与 SQL、表格界面与查看器、连接 / 秘密 / 外壳 / UI、导航器）。每组一个 `generalPurpose` 子代理，在同一条消息里全部发出，前台等待。每个子代理覆盖全部五个维度，不要按维度分子代理。

提示词模板（中文，把尖括号部分填满；“重点关注”从下面的高发问题清单里挑适用的）：

```text
你是资深 Rust / GPUI 代码审查者。只读审查，不要修改任何文件，不要运行 cargo（主代理在跑，会抢 target 锁）。可以用 git、rg、读文件。

## 背景
仓库 /Users/xhofe/code/i/navidog：用 gpui-kit 复刻 Navicat Premium 17 的 Rust workspace。分支 <分支>，相对 master 有 <N> 个提交。
审查命令：git diff master...HEAD -- <路径>；行号以 HEAD 下的文件为准。
gpui-kit 源码在 ~/.cargo/git/checkouts/gpui-kit-*/<rev>/crates，gpui 在 ~/.cargo/registry/src/*/gpui-pre-<版本>/src，确认组件行为时去读。

## 你负责的范围
<文件列表，各写一句这块是做什么的>

## 规范来源（先读）
AGENTS.md、docs/design/00-overview.md 第 4 节、CONTEXT.md
## 规格来源
<模块规格的章节>；docs/progress.md 第 3、4 节（已登记的偏差不算问题，除非实现与登记不符）

## 审查维度（每条发现标一个）
1. 业务功能与逻辑实现 2. 代码设计与架构 3. 代码可读性与规范 4. 安全性与健壮性 5. 性能与资源管理

## 重点关注
<从高发问题清单挑选，再加上这块特有的风险>

## Fowler 坏味道基线（都是判断项，仓库规则优先）
Mysterious Name、Duplicated Code、Feature Envy、Data Clumps、Primitive Obsession、Repeated Switches、Shotgun Surgery、Divergent Change、Speculative Generality、Message Chains、Middle Man、Refused Bequest。

## 输出要求（中文，最多约 1000 字）
先给 2~3 条做得好的地方。然后按严重度排序，每条格式：
`[严重度: 严重/重要/一般/建议] [维度编号] 文件:行号 — 问题一句话`
下一行给证据（引用不超过 3 行代码）和修复建议。
必须读代码确认后再报；拿不准的标“（疑似）”并写明还需要什么才能确认。只报真实问题，宁缺毋滥。
```

## 4. 逐条核实

子代理的结论可能有错。每个严重、重要结论都打开引用的代码核实；涉及 gpui、gpui-kit 行为的，去读上面路径下的源码。不成立的删掉或降级；只读代码、没有运行复现的写“读码确认”。

对每个确认的问题跑 `git blame -L <行>`：问题代码来自 `master` 上的提交，严重度记 `存量`，在报告里和本分支引入的问题分开列。

## 5. 测试失败时二分

用临时 worktree，和主目录共用 target，只重编 workspace 里的 crate：

```bash
git worktree add -f /tmp/cr-bisect <好的提交>
cd /tmp/cr-bisect && git bisect start <坏的提交> <好的提交>
git bisect run sh -c 'CARGO_TARGET_DIR=/Users/xhofe/code/i/navidog/target cargo test -p <crate> --test <测试> 2>&1 | grep -q "test result: ok"'
git bisect reset
cd /Users/xhofe/code/i/navidog && git worktree remove --force /tmp/cr-bisect
```

定位后在 worktree 里加调试输出找原因，用户的工作区保持不动。

## 6. 报告

按 SKILL.md 的格式写 CR 文件，所有状态记 `待修`，提交 `docs: record the <分支> code review and its fix list`。

回复先给结论（能不能合并、阻塞项），然后是验证结果表，再按用户要求的维度列问题，存量问题单列，最后是做得好的地方。每条写清位置、机制和修法。

## 高发问题清单

这个项目里反复出现过的问题，派子代理和自己核实时都对照：

- 按字节长度切 `&str`（`s[..kw.len()]`）再比关键字，非 ASCII 输入会 panic；应改用 `navidog_util::starts_with_ignore_ascii_case`。
- 对话框里既订阅输入框的 `PressEnter` 又设 `on_ok`：单行输入框会把回车传给对话框的 Confirm，确认执行两次。默认 `on_ok` 则是回车直接关闭、不应用。应统一用 `navidog_ui::skin::confirm_with`。
- 实体里只有一个 `task: Option<Task>` 槽：新任务覆盖旧任务会取消正在进行的加载，`busy` 永远不复位。
- `run_on`、`run_rel` 调用即开始执行：丢掉界面回调不会取消语句，忙时再提交会重复执行；缓存命中前不要创建任务。
- 所有会触发重新加载的路径（翻页、排序、筛选、刷新、重连），`set_data` 都会丢掉未提交的修改。
- 投影后的列（隐藏列）不能用来定位行；无主键表、复合主键尤其危险。
- SQL 字面量：数字不加引号、PostgreSQL 对非文本列用 LIKE、MySQL 反斜杠转义与 PostgreSQL `standard_conforming_strings`、自定义条件不加括号。
- 全局 action 处理函数执行时窗口被占用，回调里再用这个窗口（例如打开对话框）会静默失败，需要 `cx.defer`。
- `uniform_list` 的 `ScrollHandle::top_item()` 和 `bottom_item()` 恒为 0。
- 多个行映射到同一个选择（表下的字段行都对应这张表），树的焦点行会被映射回去。
- 渲染热路径（每格每帧）里的 `to_ascii_lowercase`、`String` 复制、重复归类类型。
- 测试辅助 `window.click` 沿用窗口当前的修饰键。
- progress.md 标了“完成”、却没有任何测试覆盖的新行为。
