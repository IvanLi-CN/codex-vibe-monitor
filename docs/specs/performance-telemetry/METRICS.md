# 性能指标合同

新指标的权威逐项定义是[77 项迁移映射](../../design/performance-observability-metrics.md)，标签、Histogram buckets、抽样和缺测规则是[指标合同](../../design/performance-observability.md#指标合同)。历史 SQLite 注册表保留于 Git 历史；不构成当前应用的存储或查询合同。

源 ID 共 77 项：9 项旧 collector/存储指标退役，1 项重复 publish duration 合并，其余迁移或修正真实来源口径。新增 HTTP、proxy、SQLite 分层等待、managed task 和采集质量指标由真实事件产生，不导入旧时间桶。

应用前缀 `cvm_`，时间 `_seconds`、容量 `_bytes`、事件 `_total`；app classic Histogram 与 hotpath native Histogram 独立抓取和查询。
