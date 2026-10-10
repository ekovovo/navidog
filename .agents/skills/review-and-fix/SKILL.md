---
name: review-and-fix
description: 审查当前分支相对 master 的改动并写出 docs/cr 报告，再按报告逐项修复、记录进度；附带只搬代码的大文件拆分。
disable-model-invocation: true
---

# 分支审查与修复

按用户的要求选一条路径，开工前先读完对应的文件：

- 审查本分支、出报告 → [review.md](review.md)
- 按 `docs/cr/` 里已有的报告修复 → [fix.md](fix.md)
- 把过大的文件拆成子模块，只搬代码 → [split.md](split.md)

## CR 文件

审查和修复共用一个文件：`docs/cr/<YYYY-MM-DD>-<分支名>.md`，中文，是这次审查唯一的进度来源。结构固定：

1. 开头：审查范围（`master...<分支>`，提交数、文件数、增删行）、怎么审的、结论（合并前必须处理什么）。
2. `## 1. 审查时的验证结果`：progress.md 第 5 节每项检查的结果表；测试失败时写上二分出的第一个坏提交和原因。
3. `## 2. 修复清单`：表格 `编号 | 严重度 | 维度 | 问题 | 状态`。
   - 编号从 `CR-01` 连续往下，修复中新发现的问题接着编号。
   - 严重度：`严重`、`重要`、`一般`、`建议`；`master` 上就有的问题记 `存量`。
   - 维度：`业务`、`设计`、`规范`、`健壮`、`安全`、`性能`、`测试`。
   - 状态只用 `待修`、`进行中`、`已修`、`待确认`（需要用户决定）、`暂缓`（日志里写明原因）。
4. `## 3. 问题说明`：每个严重、重要项一段：位置、机制、修法。
5. `## 4. 当前状态与待决定`：已修几项、哪些检查通过；待确认项一行一个并列出可选做法；用户的决定原样记下。
6. `## 5. 进度日志`：`时间 | 编号 | 说明`，每个提交一行：改了什么、加了哪个测试、怎样验证过。

修复清单的状态和进度日志跟着修复提交一起改，表格始终和代码一致。

## 检查命令

progress.md 第 5 节是完整清单。在 Cursor 沙箱外运行（`required_permissions: ["all"]`），cargo 命令加 `env -u CARGO_TARGET_DIR`：

```bash
cargo fmt --all --check && scripts/check-raw-px.sh
env -u CARGO_TARGET_DIR cargo clippy --workspace --all-targets -- -D warnings
env -u CARGO_TARGET_DIR cargo test --workspace
# 改了 SQL 生成或驱动时
docker compose up -d --wait && NAVIDOG_LIVE=1 env -u CARGO_TARGET_DIR cargo test -p navidog-db --test live; docker compose down
# 改了界面时，跑完逐张看 target/snapshots/
env -u CARGO_TARGET_DIR cargo test -p navidog --test snapshot
```

target 目录有锁：cargo 命令一次一个，并行的子代理只读代码。

## 提交

- 提交到当前分支；推送和开 PR 先问用户。
- 代码提交用 `fix: …`、`perf: …`、`refactor: …`，小写祈使句；CR 文件的状态和日志随代码一起提交。
- `docs/design/` 和 `progress.md` 的改动单独做一个 `docs: …` 提交（AGENTS.md：规格和代码分开提交）。
