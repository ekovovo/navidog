# 按报告修复

完成标准：清单里每一项要么 `已修`，要么 `待确认` / `暂缓` 并写明原因；全量检查通过；CR 文件、progress.md、相关规格和代码一致。

## 修复循环

顺序：CI 阻塞项 → 严重 → 重要 → 存量 → 一般、建议。同一根因的几项合成一个提交。每一项：

1. **找根因**：从入口到出口读完相关代码，再用 `rg` 在整个 workspace 搜同一类写法（不只是报告点名的那一行），在共用的函数里修一次。顺带修掉的存量同类问题写进日志。
2. **写测试**：写能在这个 bug 上失败的最小检查：纯函数的单元测试、headless UI 测试，或真实 SQLite 测试。决策逻辑能抽成纯函数的，就抽出来单测。
3. **证明测试有效**：先撤掉修复（`git stash push <源文件>`，或用脚本临时删掉那段），跑测试，必须失败；再恢复（`git stash pop`），跑测试，必须通过。
4. `cargo fmt --all`，跑这个 crate 的 clippy 和测试。
5. **更新 CR 文件**：该行改成 `已修`；日志加一行（改了什么、哪个测试、怎样证明有效）。修的过程中发现的新问题追加一行，接着编号。需要用户决定的标 `待确认`，先去做别的。
6. 按 SKILL.md 的规则提交。

做完一批：跑全量检查；更新 progress.md（验证列补上新测试，“临时偏差与遗留”增删）和受影响的规格，单独做一个 `docs:` 提交。

## 待确认项

全部收集起来，最后用 AskQuestion 一次问完：每项给 2~3 个做法，推荐的放第一个并标“（推荐）”。把决定原样记到 CR 文件第 4 节，再当普通项去修。

## headless 测试写法

参考：`crates/navidog-grid/tests/viewer.rs`、`crates/navidog-navigator/tests/keys.rs`、`crates/navidog/tests/close_prompt.rs`、`crates/navidog-connection/tests/unlock.rs`。

- 每个测试文件只放一个测试：它用 `unsafe { std::env::set_var("NAVIDOG_HOME", 临时目录) }` 改了进程环境。
- 建表：在 `navidog_db::runtime::runtime().block_on` 里用 `navidog_db::connect` 加 `SqliteDriver` 执行 SQL。断言写入结果时，用另一个连接查数据库文件。
- 连接：`ConnectionStore` 先 `set_key(Key::random())`（避开钥匙串）再 `upsert`；`cx.executor().allow_parking()`；`Sessions::open(id, Secrets::default(), cx)`。
- 等待：循环 `run_until_parked` 加 `sleep(20ms)`，10 秒超时并说明在等什么。
- 网格数据到了，`render_frame` 之后 `window.try_find(ElementId::NamedInteger("row".into(), 0))` 能找到。提交触发了重新加载的，编辑之前先空转约 20 轮等加载结束。
- 点单元格：`click_at(("row", n), 偏移)`，横向偏移是 `tokens::grid::ROW_NUMBER_WIDTH` 加若干个 `COLUMN_WIDTH`。编辑：`enter`、`cmd-a`、`input`、`enter`。
- 按钮在忙时禁用，可以在等待循环里反复点，直到效果出现。

## 性能项

修之前和修之后都量一次：`cargo run --release -p navidog-grid --example spike`（沙箱外，需要 Metal）。比较编译选项时用环境变量（如 `CARGO_PROFILE_RELEASE_OPT_LEVEL=3`），不改 `Cargo.toml`。release 加 fat LTO 要编好几分钟，期间它占着 target 锁：放到后台，先改文档。数据写进对应规格的性能小节。
