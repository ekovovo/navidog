# macOS 免费签名验收

日期：2026-10-24。分支：`codex/macos-adhoc-signing`，基于 `upstream/main` 的 `a35b796`；规格先独立提交为 `f214daa`。对应 F-REL-001、F-REL-002、F-REL-003，规格见 [150-macos-distribution.md](../design/150-macos-distribution.md)。

## 实现

- `scripts/sign-macos.sh` 在完整应用就绪后执行免费 ad-hoc 签名，并用 `--deep --strict --all-architectures` 严格验签；无需证书、网络时间戳或 `sudo`。只在验签时使用 `--deep`。
- 本地打包和无证书的 Intel / Apple Silicon 发布使用同一脚本；正式证书分支保留原签名与时间戳，也增加严格验签，不被免费签名覆盖。
- 发布冒烟改为启动签名后的 bundle 内程序。公证及 staple 必须同时具备证书与公证凭据，避免把 ad-hoc 包送去公证。
- 两个 macOS lint 任务接入真实工具回归，有三分钟超时。README 说明安装到 `~/Applications` 的权限收益及系统放行步骤。
- 不改变 Rust 更新器：继续完整复制与替换 `.app`，不在用户电脑重签、不删除 quarantine、不修改 Gatekeeper。

## 本机验证

环境：macOS 27.2（26B118），arm64，仓库固定 Rust 1.99.0。

| 检查 | 结果 |
| --- | --- |
| `scripts/test-macos-signing.sh` | 通过：真实 clang 最小应用、空格路径、签名/运行、资源篡改拒绝、重新签名、DMG 挂载/ditto 复制/整包替换，替换后 CDHash 一致且可运行 |
| `env -u CARGO_TARGET_DIR scripts/bundle-macos.sh` | 通过：生成完整应用与 DMG，`Signature=adhoc`，严格验签通过 |
| 完整应用安装/更新复制 | 从新 DMG 复制 Navidog，用 rename 先移开旧包、再换上新包，严格验签及 CDHash 比对均通过 |
| 签名后完整应用首帧 | 隔离 `NAVIDOG_HOME`，设置 `NAVIDOG_SMOKE_TEST=1`，退出码 0，出现 `NAVIDOG_SMOKE_WINDOW` 与 `NAVIDOG_SMOKE_OK` |
| Shell / 工作流检查 | 三个 shell 脚本 `bash -n`、两个工作流 YAML 解析及 `git diff --check` 通过 |
| fmt / 裸值 / clippy | 通过，Clippy 使用 workspace、all-targets、locked、`-D warnings` |
| 在线 cargo audit / deny | 通过，保留既有九项允许警告，未新增忽略项 |
| 实库模式 workspace 测试 | `NAVIDOG_LIVE=1 cargo test --workspace --locked`：284 项 / 43 套件，0 失败、0 忽略；含 15 项 DB live 与查询解释实库测试 |
| Docker 清理 | 已执行 compose down，移除本轮容器与网络 |

本地产物：`target/bundle/navidog.app`、`target/bundle/navidog-0.0.1.dmg`。DMG SHA-256：`fb19c47b1a13de257fefec93911485116f819e550e5b11a4ef0247bdd0fde4fb`。

构建、实包复制/首帧、全量检查日志位于 `target/macos-signing-*.log`；自动回归可用上述脚本重跑。安装包与临时检查文件不提交。

## 验证边界

本次没有界面或 Rust 代码变化，未新增截图及性能检查。真实签名与应用运行只在本机 arm64 验证；Intel CI 已接入，远端运行待验证。没有正式证书，Developer ID、公证和票据行为未经实测。

ad-hoc 保证完整性，不提供 Apple 认证身份。实际浏览器下载后的 Gatekeeper 放行、不同 macOS 版本和钥匙串授权不由本轮本地复制检查证明。免费方案仍可能出现系统“仍要打开”确认；不承诺所有更新都免提示。现有 `/Applications` 权限由系统决定，本轮未迁移用户的已安装应用或修改其配置。
