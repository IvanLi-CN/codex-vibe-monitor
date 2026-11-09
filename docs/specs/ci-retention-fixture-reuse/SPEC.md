# CI 旧版本迁移 fixture 复用

## Context and Scope

本主题约束 CI 中由已发布镜像生成的旧版本原始状态快照复用，减少重复准备。候选程序迁移、运行时验证、应用持久化格式、测试断言与真实时间阈值不在优化范围内。

## Requirements

### REQ-FIXTURE-CACHE-001

缓存 MUST 只保存逐个已发布镜像生成的原始 fixture，覆盖完整声明版本集；缓存键 MUST 绑定 runner OS/架构、格式版本、镜像 digest 清单、生成器、缓存实现和升级验证脚本。MUST NOT 使用前缀回退键或保存候选程序、验证结果及迁移后的状态。

### REQ-FIXTURE-CACHE-002

命中缓存后 MUST 校验输入身份、完整文件与目录清单、文件大小和 SHA-256、逐版本来源以及必需数据库，拒绝符号链接和特殊文件。身份漂移、内容损坏或不完整 MUST 阻止验证通过。未命中 MUST 按原路径逐版本生成；生成失败 MUST NOT 发布缓存。

### REQ-FIXTURE-CACHE-003

快照目录和本次工作目录 MUST 分离；候选验证 MUST 使用独立副本，并在每次运行中执行全部已声明版本的迁移、运行时与候选身份核验。缓存保存 MUST 发生在原始状态完整校验后、候选验证前。现有 required gate 和失败传播 MUST 保持原义。

### REQ-FIXTURE-CACHE-004

复用 MUST 保持相同 runner、任务数、测试覆盖及并发度，并记录缓存状态、体积及输入身份。性能结论 MUST 来自同一候选的冷、热缓存完整运行对照，同时报告关键路径和 runner 总工作量；单步命中不能替代完整 CI 验证。

## Verification

### VER-FIXTURE-CACHE-001

- covers: `REQ-FIXTURE-CACHE-001`, `REQ-FIXTURE-CACHE-002`
- 离线行为测试覆盖冷生成、热命中、各输入失效、缺失和额外文件、内容损坏、目录缺失、链接、大小边界及生成失败；工作流变异检查拒绝回退键、错误路径和不完整缓存格式。

### VER-FIXTURE-CACHE-002

- covers: `REQ-FIXTURE-CACHE-003`
- 修改工作副本中的真实 SQLite 数据后，原始缓存和下一次工作副本必须保持原始状态；机械合同检查保存顺序、验证命令与失败传播。

### VER-FIXTURE-CACHE-003

- covers: `REQ-FIXTURE-CACHE-003`, `REQ-FIXTURE-CACHE-004`
- 同一候选分别完成原生 CI 冷生成和热复用；全部迁移与运行时验证成功，记录缓存准备耗时、缓存字节及完整运行耗时。真实失效输入仍需生成，错误缓存不得形成绿色迁移证据。

## Related ADRs

None

## References

- [实现说明](IMPLEMENTATION.md)
- [主题背景](HISTORY.md)
- [CI 性能方法](../../solutions/performance/github-actions-ci-performance-optimization.md)
