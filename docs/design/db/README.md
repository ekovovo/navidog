# 其他数据库的专属功能

这里只列 MySQL、PostgreSQL、SQLite 之外的数据库所独有的功能。通用功能（连接表单、导航树、数据编辑器、查询编辑器等）以 [功能清单](../01-feature-inventory.md) 为准，各库的差异写在对应规格的“方言差异”一节。

| 数据库 | 类别 | 清单 | 里程碑 |
| --- | --- | --- | --- |
| 达梦 DM8 | Relational | [dameng/features.md](dameng/features.md) | M14 |
| SQL Server | Relational | [mssql/features.md](mssql/features.md) | M14 |
| Oracle | Relational | [oracle/features.md](oracle/features.md) | M14 |
| Snowflake | Relational | [snowflake/features.md](snowflake/features.md) | M14 |
| MongoDB | Document | [mongodb/features.md](mongodb/features.md) | M14 |
| Redis | KeyValue | [redis/features.md](redis/features.md) | M14 |

MariaDB 和 MySQL 放在同一个 crate `navidog-mysql`：两个 `Driver`，同一套方言。接入一个新数据库时新增一个不依赖 GPUI 的 crate，再在 `navidog-session` 注册。关系型数据库实现 `Driver`、`Relational` 和 `Dialect`；文档库和键值库实现对应的类别 trait（`Document`、`KeyValue`）。连接表单读 `DriverInfo` 上的标志（多主机、文件库、网络库、SSL 模式列表）。grid、query、designer、schema 只问能力方法或 `DriverInfo` 标志，不按 `Dialect::id()` 分支；`scripts/check-dialect-branches.sh` 在 CI 里检查。M14 先接关系型的达梦 DM8（规格 [140-dameng.md](../140-dameng.md)，关系型 trait 已就绪）；Oracle、SQL Server、MongoDB、Redis 仍按类别补规格后再做。

接入顺序按类别推进：先在 `navidog-db` 中实现 `Document` 和 `KeyValue` 两个类别 trait，并为各 feature crate 加上对应的界面，然后逐个接入数据库。每个数据库开工前都要补一份详细规格。
