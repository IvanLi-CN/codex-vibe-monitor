# CI fixture 复用实现

`ci-pr.yml` 的 Retention Migration Validation 使用精确输入键恢复 `retention-release-fixture-cache`，不使用回退键。`generate-retention-maintenance-release-fixtures.py --cache-dir` 通过 `retention_fixture_cache.py` 校验原始快照，并复制至当前运行的 `retention-migration-fixtures`。缺失时仍运行原有全部镜像 digest，生成完整状态后保存快照；损坏或输入不符时失败闭锁并保留诊断现场。

输入身份包含生成器、缓存模块、完整升级验证脚本和发布源清单。完整脚本哈希采用保守失效，避免版本集合或生成依赖变化后复用旧数据。清单覆盖全部目录与普通文件，限制为 4096 项、512 MiB，拒绝符号链接及特殊文件；内容 SHA-256 用于完整性检查，不宣称为独立来源签名。

缓存保存位于原始快照校验之后、迁移验证之前。升级和运行时验证器继续复制逐版本 fixture 到各自输出目录，并在本次运行中重新播种相对当前时间的测试数据。候选身份核验、全部版本、`--jobs 2`、真实数据库和运行时断言保持原样。恢复、复制、校验或保存失败均不能形成成功迁移证据；通过原生缓存作用域隔离 PR 与主干缓存。

`test-retention-fixture-cache.py` 覆盖缓存行为及工作流变异，接入既有质量门禁合同。托管平台冷、热缓存对照与具体耗时保存在绑定候选 SHA 的 PR 证据中；回退可移除缓存步骤及 `--cache-dir`，恢复完整生成路径。
