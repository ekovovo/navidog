# 达梦 DM8 接入验收记录

分支 `feat/dameng`，对应 M14。规格：[140-dameng.md](../design/140-dameng.md)、[db/dameng/features.md](../design/db/dameng/features.md)。驱动选型：`dm-pure`（MIT，纯 Rust 同步 API）为主，ODBC 兜底；阶段 0 判据 1–4 全部通过，**继续 dm-pure 路线，不切 ODBC**。

## 技术验证（阶段 0）

### 环境与许可

- 驱动：`dm-pure` 1.2.4（crates.io 最新版，MIT）。同步 API，照 `navidog-sqlite` 的阻塞线程模式跑在 `navidog-db` 运行时上。
- 服务器：DM8 官方 Docker 镜像 `dm8_single:dm8_20241022_rev244896_x86_rh6_64`，从官网 `download.dameng.com` 下载 tar（约 769 MB）后 `docker load`。x86_64 镜像，本机 Apple Silicon 经 OrbStack Rosetta 模拟运行（`--platform linux/amd64 --privileged`）。
- **许可核实**：开发版许可为 1 年试用，条款限定学习/测试/开发用途。结论：**DM 镜像不进 CI**，实库测试标记为本地可选（手动起实例 + 环境变量开关，见规格第 8 节）。
- 三个实例（凭据均 `SYSDBA/SYSDBA001`；`UNICODE_FLAG`、`CASE_SENSITIVE` 是建库参数，初始化后不可改；`COMPATIBLE_MODE` 是静态参数，`SP_SET_PARA_VALUE(2,'COMPATIBLE_MODE',n)` + 重启生效）：

| 容器 | 端口 | UNICODE_FLAG | CASE_SENSITIVE | COMPATIBLE_MODE | 用途 |
| --- | --- | --- | --- | --- | --- |
| `dm8-utf8` | 15236 | 1（UTF-8） | 1 | 0 | 主验证 |
| `dm8-gbk` | 15237 | 0（GB18030） | 1 | 0（临时切 2/4 实测后已恢复） | 编码与兼容模式 |
| `dm8-ci` | 15238 | 1（UTF-8） | 0 | 0 | 大小写不敏感 |

- 验证程序：`/tmp/dm8-verify/verify`（临时 cargo 工程，不进仓库），依赖打补丁后的 dm-pure 副本。输出存档：`result-utf8.txt`、`result-gbk.txt`（打补丁前）、`result-utf8-patched.txt`、`result-gbk-patched.txt`（四补丁后全量回归，各 51 项 PASS、0 FAIL）、`mode-utf8-cs1.txt`、`mode-utf8-cs0.txt`、`mode-gbk-mode2.txt`、`mode-gbk-mode4.txt`、`ddl-probe.txt`、`ddl-probe2.txt`。

### 复现命令

```sh
docker load -i dm8.tar
docker run -d --name dm8-utf8 --platform linux/amd64 --privileged -p 15236:5236 \
  -e PAGE_SIZE=16 -e LD_LIBRARY_PATH=/opt/dmdbms/bin -e EXTENT_SIZE=32 \
  -e BLANK_PAD_MODE=1 -e LOG_SIZE=1024 -e UNICODE_FLAG=1 -e LENGTH_IN_CHAR=1 \
  -e INSTANCE_NAME=dm8-utf8 -e SYSDBA_PWD=SYSDBA001 -e BUFFER=1000 \
  dm8_single:dm8_20241022_rev244896_x86_rh6_64
# dm8-gbk：UNICODE_FLAG=0、端口 15237；dm8-ci：CASE_SENSITIVE=0、端口 15238
cd /tmp/dm8-verify/verify
DM_PORT=15236 cargo run --bin dm8-verify   # 全量九项
DM_PORT=15236 cargo run --bin modeprobe    # 大小写/兼容模式/lazy fetch
DM_PORT=15236 cargo run --bin ddlprobe2    # 设计器 DDL/事务/SP_TABLEDEF
```

### 九项结论

| 项 | 结论 | 证据与要点 |
| --- | --- | --- |
| 1 连接与认证 | 通过 | 正常登录 OK；错误密码报 `code=-2501`「用户名或密码错误」（干净消息，见补丁 3）；版本取 `SELECT BANNER FROM V$VERSION`（`DM Database Server 64 V8`）+ `SELECT ID_CODE()`（`--03134284294-20241009-244896-20119 Pack3`，即 8.1.3.140 rev244896）。 |
| 2 目录 SQL | 通过 | `ALL_TABLES`、`ALL_TAB_COLUMNS`、`ALL_CONSTRAINTS`、`ALL_CONS_COLUMNS`、`ALL_VIEWS`、`ALL_PROCEDURES`、`ALL_INDEXES`、`ALL_IND_COLUMNS`、`ALL_OBJECTS`、`ALL_USERS`、`ALL_SEQUENCES`、`ALL_SOURCE`、`ALL_TAB_COMMENTS`、`ALL_COL_COMMENTS` 均可用。`OWNER` = 用户名（用户≈模式）；`V$DATABASE.NAME` = `DAMENG`（单数据库）；`SYS_CONTEXT('USERENV','CURRENT_SCHEMA')`、`USER`、`SF_GET_SCHEMA_NAME_BY_ID(CURRENT_SCHID())` 均返回当前模式。 |
| 3 中文与编码 | 通过 | UTF-8 与 GB18030 两实例：中文表名/列名（引用与未引用）、参数绑定与内联字面量写入读回全部精确匹配。握手 `server_encoding_name()` 返回 `UTF-8`/`gb18030`，dm-pure 用 encoding_rs 自动检测。**DM 建库的「GBK」选项实际是 GB18030**。 |
| 4 LOB | 通过（补丁 1 后） | CLOB 136000 字符、BLOB 200000 字节读回精确匹配；`set_lob_chunk_size(4096)` 同样匹配；行内小 LOB 正常。VARCHAR(10) 超长报 `-6169`「列[S]长度超出定义」，**不静默截断**。原版 1.2.4 行外 LOB 读回为空（服务器侧数据经 disql 证实已写入），根因见补丁 1。 |
| 5 事务 | 通过 | `START TRANSACTION`/`COMMIT`/`ROLLBACK` SQL 与 dm-pure 协议级 `begin()`/`commit()`/`rollback()`（cmd 52/8/9）均正常；`SAVEPOINT`/`ROLLBACK TO SAVEPOINT`/`RELEASE SAVEPOINT` 可用（`navidog_db::commit` 流程兼容）。**DDL 隐式提交确认**：事务内未提交的 INSERT 被 DDL 提交，随后 ROLLBACK 仍在（SQL 路径与协议路径各验一次）。`BEGIN TRANSACTION` 不支持（`-2004` 不支持的语句类型）。 |
| 6 取消执行 | 通过（有偏差） | dm-pure 无取消 API。第二连接 `CALL SP_CLOSE_SESSION(sessid)` 可中断长查询（40000×40000 聚合，约 8s；2s 时 kill，victim 收 `-6852` 连接关闭）。**杀的是整个会话而非仅语句**，取消后原会话必须重连。两实例均验证。 |
| 7 SSL/TLS | 不支持 | dm-pure 1.2.4 源码无 TLS 路径（源码审查）；服务器 `ENABLE_ENCRYPT=0` 已记录。`DriverInfo.supports_ssl=false`，SSL 页不显示。 |
| 8 会话 ID | 通过 | `SELECT SESSID()` 返回 BigInt，可用；`SF_GET_SESSION_ID()` 不存在（`-2207`）。 |
| 9 大小写与兼容模式 | 通过 | `CASE_SENSITIVE()` 函数可连接后探测（`V$DM_INI` 无此行）；`COMPATIBLE_MODE`/`ENABLE_ENCRYPT`/`BLANK_PAD_MODE` 经 `V$DM_INI` 探测。CS=1：未引用标识符折大写、引用标识符大小写敏感；CS=0：按原样存储、引用标识符不敏感匹配。mode 2（Oracle）：空串 `IS NULL`；mode 0/4：空串非 NULL。mode 4（MySQL）：支持反引号；mode 0/2 不支持（`-2007`）。 |

### 附加验证

- **分页**：`LIMIT n OFFSET m`、`TOP n`、`ROWNUM <= n`、`FETCH FIRST n ROWS ONLY` 四种语法均可用（mode 0 实测）。方言分页统一用 `LIMIT … OFFSET …`。
- **EXPLAIN**：返回缩进文本计划（简单查询 4 行）。原版 1.2.4 挂起至超时，根因见补丁 2。
- **lazy fetch**：`execute_lazy` 首批 100 行，`fetch_more` 续取 4900 行合计 5000，两实例一致；网格分批读取可用。
- **模式切换**：`SET SCHEMA <name>` 与 `ALTER SESSION SET CURRENT_SCHEMA = <name>` 均可用（补丁 4 后），`SYS_CONTEXT` 证实切换生效；无效模式名报干净服务端错误 `-2103`。
- **表 DDL 来源**：`CALL SP_TABLEDEF('SYSDBA','T')` 返回完整 `CREATE TABLE`（含引号、`STORAGE(ON "MAIN", CLUSTERBTR)`、`\r\n` 换行）。`DBMS_METADATA.GET_DDL` 对 TABLE 报 `-20008`、对 VIEW/INDEX/SEQUENCE 返回空串，**不可用**；视图 DDL 用 `ALL_VIEWS.TEXT` 拼接，索引/序列 DDL 由目录合成。
- **设计器 DDL 语法**（`ddl-probe.txt`/`ddl-probe2.txt`）：`ALTER TABLE … ADD (col …)`、`ADD col …`、`MODIFY col …`、`MODIFY (col …)`、`RENAME COLUMN a TO b`、`DROP COLUMN c`、`ALTER COLUMN c SET DEFAULT …`/`DROP DEFAULT`、`ALTER TABLE … RENAME TO new`、`COMMENT ON TABLE/COLUMN`、`ADD CONSTRAINT … UNIQUE/CHECK`、`DROP CONSTRAINT`、`CREATE/DROP INDEX`、`IDENTITY(1,1)`、`CREATE SEQUENCE … START WITH … INCREMENT BY …`、`seq.NEXTVAL` 均通过。独立 `RENAME old TO new` 不支持（`-2007`），改名统一走 `ALTER TABLE … RENAME TO`。

### dm-pure 缺陷与补丁（4 处）

对照官方 Go 驱动（`gitee.com/chunanyong/dm`，GitHub 仓库 404）协议实现定位。干净补丁 `/tmp/dm8-verify/dm-pure-pr.patch`（122 行，仅 `src/client.rs`，无调试代码）；托管仓库 `/tmp/dm8-verify/dm-pure-host`：`main` = `0a299ff`（crates.io 1.2.4 原版镜像，溯源上游 commit `58579165765aa764566c191d3a458b03c28db2f0`），`fix/protocol-lob-explain-error-status` = `59abd96`（仅四补丁，`src/client.rs` +70/-6），PR 描述 `/tmp/dm8-verify/PR_DESCRIPTION.md`。四补丁后 UTF-8 与 GB18030 全量回归各 51 项 PASS、0 FAIL（`result-utf8-pr.txt`/`result-gbk-pr.txt`）。

1. **行外 LOB 读回为空**（核心项）：`read_offrow_lob` 的 cmd=32 请求把 offset/length 写成 i64，协议要求 int32（Go `dm_build_676.dm_build_421`），服务器读到 length=0。改为 i32；CLOB 偏移按服务器返回的字符数推进（原按请求字节数）。
2. **EXPLAIN 挂起**：响应是 cmd=149 帧（Go `retSqlType` Dm_build_386=127+22=149），布局 int32 长度 + 计划文本，服务器随后不再发帧。dm-pure 只认 187/163，苦等至 EAGAIN。加 149 分支：解析文本按行返回单列 `PLAN` 结果集。
3. **错误消息带前缀**：错误体布局 12 字节头 + int32 长度 + 文本；原实现整体解码，长度低字节恰为可打印字符时出现 `E   ` 之类前缀。按布局解析后消息干净（如「用户名或密码错误」）。
4. **SET SCHEMA / ALTER SESSION 挂起**：这类会话状态语句的响应只有 cmd=0 短状态帧（length≤30）、没有 cmd=187 结果帧；exec 循环苦等 187 至 EAGAIN。循环遇到无 187 伴随的 cmd=0 短帧即停止，按 `Affected(0)` 返回；有 187 时不受影响（参数化路径回归通过）。

### 判据与路线

判据 1–4 全部通过（LOB 经补丁 1）→ **继续 dm-pure 路线**。5–9 的缺口（取消杀会话、无 SSL、DDL 隐式提交、GB18030 命名）不阻塞，逐条写进 [140-dameng.md 第 7 节「偏差」](../design/140-dameng.md#7-偏差)。

**依赖策略（已确认，上游消失后更新）**：上游仓库 `zhangtao103239/dm8_pure_rs` 在 GitHub 返回 404，全站搜索（含 fork）未找到镜像，无法 fork/提 PR。改为自建托管：用户创建公开仓库 `github.com/ekovovo/dm8_pure_rs`（非 fork，不初始化 README/license），`main` 为 crates.io 1.2.4 原版镜像，`fix/protocol-lob-explain-error-status` 分支仅含四补丁。上游发布修复版前，`navidog-dameng` 用 `[patch.crates-io]` git 依赖指向该修复分支（`dm-pure = "=1.2.4"` + patch，提交 Cargo.lock）；上游恢复后向其提 PR，上游发修复版后切回 crates.io 精确版本并移除 patch。

## 验收记录（阶段 2/3，2026-11-24）

### 实现

- **`crates/navidog-dameng`**（不依赖 GPUI，`check-deps.sh` 通过）：`dialect.rs`（`DamengDialect { case_sensitive, compatible_mode }`，能力字段随探测结果切换）、`words.rs`（`V$RESERVED_WORDS` 的 231 个保留字减 COMMON 共 159 个 extra + 内置函数表）、`metadata.rs`（八个目录方法经 `relational_catalog!` 转发）、`driver.rs`（照 `navidog-sqlite` 的阻塞线程 + watch 退休栅栏；`recovering_execute!`/`recovering_ping!`；取消经短期连接 `CALL SP_CLOSE_SESSION`，偏差 D2）、`locales/dameng.yml`（en/zh-CN）。
- **共享层接线**：`navidog-schema` 的 `plan.rs` 调用 `add_column`（DM 括号形式）与 `modify_column`（DM 接管整段序列；含 IDENTITY 子句的变化报错拒绝）；PG/MySQL/SQLite 回归不变。`navidog-session::init` 注册第五个内置驱动；`check-deps.sh` 名单加 `navidog-dameng`；自绘抽象图标 `icons/db/dameng.svg`（原创图形，未取材任何既有 logo）。
- **用户工具只读列表（规格 6.5）**：`Unavailable` 且 `account_list_sql()=Some` 时渲染只读列表——隐藏「新建」「设计」、禁用双击打开、保留筛选刷新，原因文本显示在工具栏；SQLite（无列表 SQL）仍显示原因文本。`tools.rs` 两处硬编码 `SQLITE_AUTH_REASON` 改为 `Unavailable(reason)` 携带的原因。
- **迁移不可回滚提示（规格 6.2）**：预览页在 `transactional_ddl=false` 时显示固定提示（en/zh-CN）。注意：MySQL 同样是 `transactional_ddl=false`，预览页因此新增同一提示——能力驱动、事实正确（MySQL DDL 亦隐式提交），超出原规格范围的界面变化记录在 progress.md 偏差表。

### 阶段 2 实测对规格的修正

- `ALL_TAB_COLUMNS` **没有** `IDENTITY_COLUMN` 列，IDENTITY 种子/增量不在任何目录列。可用来源：`SYS.SYSCOLUMNS.INFO2`（identity 列为 1）+ 系统函数 `IDENT_SEED`/`IDENT_INCR`（接受 `'模式.表'` 字符串；非 identity 表报 -7019，且 DM 在 CASE 标量子查询里会**预先求值**所有分支，WHERE 守卫无效）。最终实现：identity 文本由 `column_metadata_sql` 的 `UNION ALL` 分支产出，该分支按 `S.INFO2 = 1` 过滤行，非 identity 表在分支里没有行、函数不执行。已写回规格 5.4。
- `ALL_SOURCE` 的 `TYPE` 把函数记为 `'PROC'`（不是 Oracle 的 `'FUNCTION'`）；DDL 过滤用 `TYPE IN ('FUNCTION', 'PROC')`。
- `ALL_TRIGGERS` 的种类列是 `TRIGGERING_TYPE`。
- `SP_TABLEDEF` 不能作为 SELECT 项（-4015），只能 `CALL`；驱动把 `ExecuteResult::Call` 的 OUT 值映射成一行结果集。
- 目录查询用 `string_literal` 引号字面量过滤（`execute_read` 无参数通道），照 PG 惯例；规格第 4 节「绑定参数」的表述按此调整。
- `LISTAGG` 可用于拼接 `ALL_SOURCE` 多行文本。
- 取消探测：`SELECT COUNT(*)` 形式的笛卡尔积被 DM 优化掉（1.6e9 行 375ms），取消测试改用 `SUM(A.N*2+B.N)` 强制求值（6.6s）。

### 测试

- 单元测试：dameng 23 项（方言引用/折叠/分页/事务 SQL/能力随 CS 与 mode、目录 SQL 文本、类型映射、错误码映射、words 非空）；`navidog-schema` 新增 DM 迁移计划 4 项（ADD 括号形式、RENAME+MODIFY 序列、IDENTITY 拒绝、仅默认值变化走 `ALTER COLUMN … DROP DEFAULT`）；`security_plan` 补 Dameng 只读列表契约；`navidog-headless` 新增 dameng 编译回归（驱动栈不链接 GPUI、DriverInfo 标志与规格一致）。
- 实库测试（`NAVIDOG_DM_LIVE=1`，本地 dm8-utf8 实例，10 项全部通过）：连接与版本（BANNER + ID_CODE）、错误密码 `-2501` 干净中文消息、目录八方法（含 SP_TABLEDEF DDL、identity 富集文本 `IDENTITY(1, 1)`）、中文表名/列名/数据、LOB（136000 字符 CLOB + 200000 字节 BLOB 参数化写入读回）、事务与 DDL 隐式提交、取消杀会话后新连接（SESSID 变化）、SET SCHEMA、`LIMIT … OFFSET` 分页、EXPLAIN 单列文本、lazy fetch 首批 100 行续取至 5000、设计器迁移语法 17 条（ADD/MODIFY/RENAME COLUMN/DROP COLUMN/SET DEFAULT/DROP DEFAULT/约束/注释/索引/改表名）。
- **补丁依赖的自动跳过**：未打补丁的 dm-pure 1.2.4 上，LOB 行外读回为空（补丁 1）、EXPLAIN 挂起（补丁 2）、SET SCHEMA 挂起（补丁 4）、`SP_CLOSE_SESSION` 的 CALL 响应挂起（同补丁 4 形状）——实库测试对前三者检测到该形态时打印原因并跳过；补丁接线后守卫自动转正。本轮实测中取消杀会话本身立即生效（服务器侧），仅取消连接的响应等待超时。
- 全量检查通过：`cargo fmt --check`、`clippy --workspace --all-targets -D warnings`、`check-deps.sh`、`check-dialect-branches.sh`、`check-i18n.sh`、`check-raw-px.sh`、`cargo test --workspace`（69 套件）、截图测试（既有方言截图无变化）。

### 遗留

1. dm-pure 四补丁接线（见 progress.md 偏差表）：重建补丁 → 用户创建 `github.com/ekovovo/dm8_pure_rs` → 推送（征得同意）→ 根 Cargo.toml `[patch.crates-io]` git 依赖 → 重跑实库测试转正三个跳过守卫 → 提交 Cargo.lock。
2. DM 界面整体走查（连接表单/导航树/表查看器/查询编辑器/表设计器浅深色截图）需本机连 DM 实例目测；CS=0 与 COMPATIBLE_MODE=2/4 实例的引用与空串行为按能力字段切换，留待下一轮。
3. 规格验收清单中依赖以上两项的条目未勾选，其余见 [140-dameng.md §8.2](../design/140-dameng.md#82-验收清单)。

