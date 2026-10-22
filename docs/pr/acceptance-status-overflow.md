# 批量删除 SQL 状态栏溢出修复验收

日期：2026-10-22。基线：`upstream/main` 的 `4c403de`。先按用户要求在本地 `main` 修复显示问题，再从 `main` 拆出 `codex/fix-batch-sql-footer-audit`，连同依赖审计修复提交 PR。规格先提交，代码与验证记录另行提交。

## 问题与修复

批量提交把多条 SQL 用 `;\n` 拼成状态文字。GPUI 的 `whitespace_nowrap` 只限制自动换行，仍会对显式换行排出多行。修复前，回归测试中三条 DELETE 的文字高度为 48px，超出底栏的 28px 高度。

共享的 `skin::truncated` 只对显示文字把 CR/LF 换成空格，保留原文给悬停提示，并明确限制为一行。表查看器与查询结果底栏固定高度、允许文本区域收缩。查询结果传入原始 SQL，避免在显示前丢失悬停原文的格式。SQL 执行、事务及删除逻辑保持原有流程。

## 依赖审计修复

基线审计发现两项 TLS 证书信任相关漏洞，按 RustSec 公告升级到已修复的补丁版本：

| 依赖 | 原版本 | 修复版本 | 公告 |
| --- | --- | --- | --- |
| `rustls-webpki` | 0.103.15 | 0.103.16 | [RUSTSEC-2026-0382](https://rustsec.org/advisories/RUSTSEC-2026-0382.html) |
| `webpki-roots` | 1.0.9 | 1.0.10 | [RUSTSEC-2026-0381](https://rustsec.org/advisories/RUSTSEC-2026-0381.html) |

仅更新 `Cargo.lock` 中这两个包的版本和校验值。现有依赖范围兼容，无需修改 `Cargo.toml`、审计规则或忽略项。`cargo metadata --locked` 确认锁文件可直接使用。

## 检查记录

| 检查 | 结果 |
| --- | --- |
| `cargo test -p navidog-ui --test truncated --locked` | 修复前可复现溢出；修复后通过。浅色及深色分别覆盖 LF、CRLF 和长中文/emoji SQL，断言单行高度、底栏边界及右侧按钮位置 |
| `NAVIDOG_LIVE=1 cargo test --workspace --locked` | 203 项通过，无失败或忽略；包含 MySQL、MariaDB、PostgreSQL、SSH、查询编辑器与执行计划实库测试 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过；间接依赖仍有 future-incompatibility 提示 |
| `cargo fmt --all --check`、`scripts/check-raw-px.sh`、`git diff --check` | 通过 |
| `cargo test -p navidog --test snapshot --locked` | 通过，逐张检查本次生成的 33 张 Metal/headless 截图；新增 6 张真实批量删除的浅深色、窄内容区和悬停截图 |
| `cargo run --release -p navidog-grid --example spike --locked` | 10 万行 × 20 列：逐行/跳跃滚动平均 2.77/3.73ms，600 帧均未超过 8.3ms |
| `cargo run --release -p navidog-navigator --example tree_spike --locked` | 逐行/跳跃滚动平均 0.52/0.71ms，600 帧均未超过 8.3ms |
| `cargo run --release -p navidog-query --example editor_spike --locked` | 5 万行 SQL：滚动、带装饰滚动、连续输入平均 1.49–1.57ms，1200 帧均未超过 8.3ms |
| `cargo audit`、`cargo deny --all-features check` | 更新后均通过：deny 的 advisories、bans、licenses、sources 全部通过；audit 仍有 9 条策略允许的间接依赖 unmaintained 警告，原有 `rsa` 例外保持不变，未新增忽略项 |

实库检查前启动仓库的 Docker Compose 测试服务，检查后已关闭。所有数据库交互与截图使用独立测试数据，没有操作用户的连接或数据库。

## 截图场景

截图程序在临时 SQLite 表 `wide` 中用真实鼠标点击、Shift+Down、删除和应用按钮一次删除两行，等待网格清空后再截图。关闭“修改后同步记录”，以保留本次 DELETE 状态，避免被刷新时的 SELECT 覆盖。截图后恢复设置。

已核对栏内两条 DELETE 连成一行并以省略号结束，悬停提示仍分两行完整显示；打开信息窗格后可用宽度变窄时，状态文字继续截断，底栏不向上下溢出。浅色、深色、悬停和窄内容区的原图在 PR 描述里，不进仓库。

本机 Metal/headless 检查不等同于实际 AppKit 窗口的人工操作验收。
