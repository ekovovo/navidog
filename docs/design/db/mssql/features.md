# SQL Server 专属功能

| 编号 | 功能 | 摘要与验收要点 | 帮助页 |
| --- | --- | --- | --- |
| F-MSSQL-001 | 连接常规设置 | 主机、端口、验证方式（SQL Server 验证、Windows 验证、Azure AD），初始数据库。 | con_gen_settings_sqlserver |
| F-MSSQL-002 | 连接参数 | 在“高级”页填写 ODBC 风格参数，如 `ApplicationIntent=ReadOnly`；执行超时。 | con_adv_settings |
| F-MSSQL-003 | Azure SQL 防火墙规则 | 右键“SQL Azure 防火墙规则”，按 IP 范围添加规则。 | connection |
| F-MSSQL-004 | 数据库与模式 | 新建、编辑数据库（文件组、文件）和模式。 | sqlserver_database |
| F-MSSQL-005 | 表设计器差异 | 唯一约束、存储（文件组、分区方案）、标识列、计算列。 | sqlserver_table |
| F-MSSQL-006 | 视图、过程、函数 | 视图（含架构绑定）、存储过程、标量与表值函数。 | sqlserver_view, sqlserver_function |
| F-MSSQL-007 | 其他对象 | 同义词、程序集、序列、用户定义类型等。 | sqlserver_others |
| F-MSSQL-008 | 维护 | 重建、重组索引，更新统计信息，检查数据库。 | sqlserver_maintain |
| F-MSSQL-009 | 安全性 | 登录名、服务器角色、数据库用户、数据库角色、应用程序角色设计器。 | security_sqlserver_login, security_sqlserver_server_role, security_sqlserver_db_user, security_sqlserver_db_role, security_sqlserver_app_role |
| F-MSSQL-010 | 原生备份与还原 | 调用 SQL Server 自身的 BACKUP/RESTORE（完整、差异、事务日志），与内置备份并存。 | backup_sqlserver, restore_sqlserver, backup_restore_sqlserver |
