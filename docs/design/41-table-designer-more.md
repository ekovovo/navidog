# M4 规格：表设计器后续

覆盖 F-DSG-005、F-DSG-006、F-DSG-008，以及 [40-table-designer.md](40-table-designer.md) 里推迟的迁移。字段拖放、引擎私有选项表单、运行态并排不在本文。布局沿用 40 第 8 节的 FormRow（300pt 输入、22pt 控件、8pt 间距）和 `tokens.designer`，不新增尺寸。

页签加在现有「检查」和「选项」之间：排除 [PG]、触发器、分区 [My][PG]。方言没有该能力时不显示这一页。SQL 预览仍只读，执行失败保留草稿。

## 1. 排除约束 [PG]

列表加表单：名称、元素（每行「表达式 WITH 运算符」，至少一个）、方法（缺省 `gist`）、可选 WHERE。保存生成：

```sql
ALTER TABLE t ADD CONSTRAINT name EXCLUDE USING gist (expr WITH operator) WHERE (predicate)
```

新表写进 `CREATE TABLE` 的约束列表。删掉已有约束生成 `ALTER TABLE t DROP CONSTRAINT name`。表达式和 WHERE 用 40 第 3 节的单行 SQL 编辑器（草稿字段名和条件关键字）。解析不了的已有 `EXCLUDE` 留在高级定义，不猜测改写。

## 2. 触发器

列表加表单：名称、时机（`BEFORE` / `AFTER`；PostgreSQL 和 SQLite 另有 `INSTEAD OF`，只在视图上提供，表上不提供）、事件（`INSERT` / `UPDATE` / `DELETE`，可多选）、对每一行（PostgreSQL 可关，关了是 `FOR EACH STATEMENT`；MySQL 和 SQLite 固定为行级）、定义。

- MySQL / MariaDB：`CREATE TRIGGER name timing event ON table FOR EACH ROW body`。`body` 是一条语句。
- PostgreSQL：`CREATE TRIGGER name timing event ON table FOR EACH ROW EXECUTE FUNCTION fn()`，定义框填写 `EXECUTE` 及其后的片段。不在这里新建被调函数。
- SQLite：`CREATE TRIGGER name timing event ON table BEGIN body END`。

已有触发器按名称替换：先 `DROP TRIGGER`，再 `CREATE`。PostgreSQL 和 SQLite 的 `DROP TRIGGER` 带表名。名称对不上或正文解析不了时保留原文并阻止结构化保存。触发器随表快照里的 `triggers` 加载。

## 3. 分区 [My][PG]

表单：方式（`RANGE` / `LIST` / `HASH`；MySQL 另有 `KEY`）、键（列或表达式）、分区列表。每一项有名称和边界：

- `RANGE`：`VALUES LESS THAN (bound)`（MySQL）或 `FROM (a) TO (b)`（PostgreSQL）。
- `LIST`：`VALUES IN (list)`。
- `HASH` / `KEY`：MySQL 用分区个数；PostgreSQL `HASH` 用 `MODULUS n REMAINDER r`。

MySQL 写在 `CREATE` / `ALTER TABLE … PARTITION BY` 子句里。PostgreSQL 父表是 `PARTITION BY method (key)`，每个分区是 `CREATE TABLE name PARTITION OF parent FOR VALUES …`，和父表改动放在同一事务。已有分区表只改列表时，能 `ATTACH` / `DETACH` 的按名称做；方式或键变了明确拒绝，不重建父表。SQLite 没有这一页。

## 4. 迁移

仍先预览再执行。预览在语句前加一行注释，说明这是整表重建；转成 `WITHOUT ROWID` 时再写明隐式 rowid 会丢掉。注释不执行。

- PostgreSQL 现有列顺序变化：同一事务里 `CREATE TABLE` 临时名（列按新顺序，含约束）、按 `original` 列名 `INSERT … SELECT`、`DROP TABLE` 旧表（不用 `CASCADE`）、临时表改回原名、重建独立索引、再写注释和拥有者。入站外键或视图挡住 `DROP` 时整笔回滚，错误原文留下。不在表达式里替换标识符。
- identity：只改了 `GENERATED … AS IDENTITY` 子句时，添加用 `ADD GENERATED {ALWAYS|BY DEFAULT} AS IDENTITY`，删除用 `DROP IDENTITY`，模式切换仍用 `SET GENERATED`。括号里的序列选项必须与原来相同，否则拒绝。和其他列改动一起出现时，先做类型 / NULL / DEFAULT，再做 identity。
- SQLite 生成列：重建时把生成列写进 `CREATE`，`INSERT` 不包含它们。表达式原样拷贝，不求值。
- SQLite 已有表切换 `WITHOUT ROWID`：允许。不拷贝隐式 `rowid`。普通表重建仍拷贝一个未占用的 rowid 别名。
- SQLite 重命名可以和其他结构改动一次保存。拷贝用 `original` 到新名。索引和触发器按快照里的语句在新表上重建；语句里出现旧表名时只替换表名，不改写列名。替换后仍不像一条 `CREATE INDEX` / `CREATE TRIGGER` 时拒绝。
- 仍然拒绝：解析不了的字段或约束、未知表选项、删除仍被索引 / 视图 / 触发器引用的字段（保存点里的 `DROP COLUMN` 预检不变）、附加库上的表重建。

## 5. 验收

- [ ] 排除约束可添加、修改、删除，PostgreSQL 实库往返。
- [ ] 三种库的触发器可添加和替换，失败回滚或保留草稿。
- [ ] MySQL 与 PostgreSQL 的分区方式和列表可预览并执行；改方式被拒绝。
- [x] PostgreSQL 列重排在事务内完成，有依赖时整笔失败。
- [x] identity 可添加、删除和切换模式；改序列选项被拒绝。
- [x] SQLite 生成列、`WITHOUT ROWID` 和合并重命名保留数据，预览写明重建。
- [x] 未知选项、依赖字段、附加库重建仍被拒绝。
