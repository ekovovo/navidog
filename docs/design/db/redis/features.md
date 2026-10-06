# Redis 专属功能

Redis 属于 `KeyValue` 类别，导航树以数据库编号和键前缀分组，数据编辑器按键类型切换界面。

| 编号 | 功能 | 摘要与验收要点 | 帮助页 |
| --- | --- | --- | --- |
| F-REDIS-001 | 连接常规设置 | 连接方式（独立、集群、哨兵），主机、端口、验证（密码、ACL 用户）。 | con_gen_settings_redis |
| F-REDIS-002 | 哨兵设置 | 哨兵节点列表、哨兵组、哨兵 SSL。 | con_sentinels_settings |
| F-REDIS-003 | 兼容性设置 | 兼容性选项。 | con_compatibility_settings |
| F-REDIS-004 | 数据库 | 数据库 0 到 N，显示键数量。 | redis_database |
| F-REDIS-005 | 数据编辑器 | 按键类型（String、List、Hash、Set、Sorted Set、Stream、JSON 等）显示和编辑；TTL 设置；新增键。 | redis_data, data_redis, data_edit_redis |
| F-REDIS-006 | 搜索与排序键 | 按模式（SCAN）搜索键，排序键列表。 | data_search_redis, data_sort_redis |
| F-REDIS-007 | 导航栏 | 分批加载键。 | data_navigation_bar_redis |
| F-REDIS-008 | 查询 | 命令编辑器和查询结果。 | query_results_redis |
| F-REDIS-009 | Pub/Sub | 订阅频道、频道颜色、Pub/Sub 窗口、信息窗格中的活跃频道。 | pub_sub, pub_sub_channel, pub_sub_color, pub_sub_window |
| F-REDIS-010 | 用户管理 | ACL 用户与规则管理。 | security_redis |
| F-REDIS-011 | 备份与还原 | Redis 专用备份文件 `.nbr`，内置备份还原工具。 | backup_redis, restore_redis, build_in_backup_restore_redis |
| F-REDIS-012 | 命令监控 | 实时查看 MONITOR 输出。 | command_monitor |
