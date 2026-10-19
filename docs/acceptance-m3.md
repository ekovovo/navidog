# M3 本机验收记录

日期：2026-10-18。基线：`upstream/main` 的 `24daa3f`；开发分支：`codex/m3-completion`。本文记录本次实际执行的检查，不替代模块规格的未完成清单。

## 本次完成

- 命令列的表头、数据和数字补空格统一按 Unicode 显示宽度计算，修复中文、全角字符和组合字符造成的错位；复用锁文件已有的 `unicode-width 0.2.2`。
- SQLite 编辑器补充运行选区、运行当前语句、21 条语句只显示 20 个结果、取消慢查询并跳过后续语句的回归检查。
- MySQL 8.4、MariaDB 11、PostgreSQL 16 的编辑器实库测试覆盖选区、当前语句、出错停止、出错继续、错误诊断和 ⌘. 取消。取消前查询服务端活动语句，避免只测到尚未开始执行时的停止。
- MySQL 和 MariaDB 实际创建、调用并删除测试存储过程，验证 `DELIMITER $$` 和 `-- NAME:`。
- 命令列通过真实按键验证多行续行、上下翻历史、取消递归查询，以及取消后继续执行。测试配置写入独立临时目录。

## 验证结果

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --all --check`、裸值检查、`git diff --check` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过；依赖 `block`、`proc-macro-error2` 有 future-incompatibility 提示 |
| `cargo test --workspace --locked` | 199 项通过；实库用下面单独开启环境变量的命令验证 |
| `NAVIDOG_LIVE=1 cargo test -p navidog-query --locked` | 27 项通过，包含服务端计划解析和新增编辑器实库测试 |
| `NAVIDOG_LIVE=1 cargo test -p navidog-db --test live --locked` | 11 项通过 |
| `cargo deny --all-features check` | advisories、bans、licenses、sources 全部通过；有重复依赖和未使用允许项警告 |
| `cargo audit` | 通过；9 条已允许的间接依赖 unmaintained 警告 |
| `cargo test -p navidog --test snapshot --locked` | 通过，检查本次生成的 27 张截图（包括查询和历史的浅深色、SQLite 解释视图） |

性能程序全部实际执行。并行编译期间首次网格测量受负载影响，不用作基线；编译结束后直接重新运行已构建的网格程序：10 万行 × 20 列，逐行滚动平均 3.48ms、p95 4.82ms；跳跃平均 5.35ms、p95 7.47ms，3/300 帧超过 8.3ms。导航树逐行/跳跃平均 0.54/0.72ms。5 万行 SQL 编辑器首次解析约 346ms，四个输入/滚动场景平均 1.62~1.71ms，均 0/300 帧超过 8.3ms。这些是本机本次测量，不是所有机器的保证。

## 仍待完成

M3 继续保持“进行中”，不把自动测试当作 Navicat 并排目测。

1. 30 第 13 节：三种数据库编辑器的浅深色、折叠、括号、换行和缩放对照。
2. 31 第 11 节：四种数据库解释的各视图、阈值变红的实际呈现。计划解析和 PostgreSQL 成本合计已有本次实库证据，完整视图目测未完成。
3. 31 第 11 节：消息页的措辞并排核对。

已从官方下载安装 Navicat Premium 17.3.12 到 `~/Applications/Navicat Premium.app`，用户明确同意 EULA 后挂载。已打开软件并创建仅指向本机测试端口的连接。桌面控制随后反复返回 ScreenCaptureKit `-3811` / “The screen capture failed”，无法可靠观察和继续对照；因此未更改语法配色，也没有勾选上述三项。Navicat 安装包、应用资源和截图不进仓库。
