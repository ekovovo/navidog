# 其他数据库的专属功能

这里只列 MySQL、PostgreSQL、SQLite 之外的数据库所独有的功能。通用功能（连接表单、导航树、数据编辑器、查询编辑器等）以 [功能清单](../01-feature-inventory.md) 为准，各库的差异写在对应规格的“方言差异”一节。

| 数据库 | 类别 | 清单 | 里程碑 |
| --- | --- | --- | --- |
| SQL Server | Relational | [mssql/features.md](mssql/features.md) | M14 |
| Oracle | Relational | [oracle/features.md](oracle/features.md) | M14 |
| Snowflake | Relational | [snowflake/features.md](snowflake/features.md) | M14 |
| MongoDB | Document | [mongodb/features.md](mongodb/features.md) | M14 |
| Redis | KeyValue | [redis/features.md](redis/features.md) | M14 |

MariaDB 归入 MySQL 驱动，差异在各模块规格的方言一节中说明。

接入顺序按类别推进：先在 `navidog-db` 中实现 `Document` 和 `KeyValue` 两个类别 trait，并为各 feature crate 加上对应的界面，然后逐个接入数据库。每个数据库开工前都要补一份详细规格。
