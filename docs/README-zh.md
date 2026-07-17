# Cockpit Simulator 中文文档索引

## 主文档

### [Cockpit Desktop 用户指南](./user-guide-zh.md)

本文是桌面端场景操作的主维护入口，覆盖：

- 加载、运行、观察和评测场景的通用操作流程；
- 10 个标杆场景各自的场景文件、目标、截止 tick、操作步骤与关键处置；
- 每个场景配套的 Mermaid 时序图与左右布局流程图，标注 `SimulationSourcePanel`、`SimulationProgress`、`SimulationWorldView`、`SimulationActivityFeed`、`SimulationEvaluation` 等实际桌面组件；
- 完成一次场景运行后的结果检查清单。

> 系统架构、通信协议、Hermes ACP 集成原理、录制回放内部机制、原生验收清单等开发/运维向内容不在本文档范围内；如需了解这些内容，请查阅相应源码目录（`crates/cockpit-runner`、`crates/cockpit-agent-runtime`）及内部注释。
> 最新 Desktop Live 只使用真实 `iota-core-acp` 模型路径；`RuleAgent` 仅保留于 CLI、Runner 合约和测试，不是 Desktop fallback。

## 专题文档

- [NPC 世界建模](./npc.md)

**最后更新时间**：2026-07-17
**统一文档版本**：5.0.0（同步场景操作指南重写后的文档结构）
