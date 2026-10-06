# MongoDB 专属功能

MongoDB 属于 `Document` 类别，数据编辑器、查询、设计器都需要文档模型专用的界面。

| 编号 | 功能 | 摘要与验收要点 | 帮助页 |
| --- | --- | --- | --- |
| F-MONGO-001 | 连接常规设置 | 连接方式（独立、副本集、分片集群、SRV）、验证方式与数据库、通过 MongoDB URI 新建。 | con_gen_settings_mongodb, connection |
| F-MONGO-002 | 高级设置 | 可重试读取、可重试写入、服务器选择超时等。 | con_adv_settings |
| F-MONGO-003 | SSL 差异 | 客户端密钥密码、允许任何服务器证书、允许无效主机名。 | con_ssl_settings |
| F-MONGO-004 | 数据库、集合、视图 | 新建、编辑数据库、集合（含时间序列集合、校验规则）和视图。 | mongodb_database, mongodb_collection, mongodb_view |
| F-MONGO-005 | 索引 | 单字段、复合、文本、地理、TTL 等索引的设计。 | mongodb_index |
| F-MONGO-006 | 函数、MapReduce、GridFS | 存储的 JS 函数；MapReduce 作业；GridFS 文件桶浏览与上传下载。 | mongodb_function, mongodb_mapreduce, mongodb_gridfs |
| F-MONGO-007 | 数据编辑器视图 | 网格视图、树视图、JSON 视图三种视图切换；类型颜色高亮。 | data_mongodb, data_grid_mongodb, data_tree_mongodb, data_json_mongodb, information_pane |
| F-MONGO-008 | 筛选、排序、查找 | 文档筛选（含“存在”“是字段类型”运算符）、排序、查找替换、格式设置、导航栏。 | data_filter_mongodb, data_sort_find_mongodb, data_format_mongodb, data_navigation_bar_mongodb |
| F-MONGO-009 | 查询与创建工具 | Shell 语法查询编辑器、查找创建工具、聚合创建工具、查询解释、查询结果。 | query_command_editor, query_find_builder, query_agg_builder, query_explain_mongodb, query_results_mongodb |
| F-MONGO-010 | 聚合管道 | 管道对象：分阶段编辑、每阶段预览、管道解释、管道结果，保存为 `.nagg`。 | agg_pipeline, agg_pipeline_builder, agg_pipeline_explain, agg_pipeline_result |
| F-MONGO-011 | 维护 | 压缩、验证、重建索引等。 | mongodb_maintain |
| F-MONGO-012 | 安全性 | 用户、角色设计器。 | security_mongodb_user, security_mongodb_role |
| F-MONGO-013 | 工具 | MongoDump、MongoRestore、MongoImport、MongoExport 的图形界面。 | mongo_dump_restore, mongo_dump, mongo_restore, mongo_import_export, mongo_import, mongo_export |
| F-MONGO-014 | 命令监控 | 实时查看服务器上执行的命令。 | command_monitor |
| F-MONGO-015 | 备份差异 | NoSQL 备份与还原。 | backup |
