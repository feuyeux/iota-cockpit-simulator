# Cockpit Simulator 中文文档索引

## 主文档

### [Cockpit Desktop 用户指南](./user-guide-zh.md)

本文是桌面端场景操作的主维护入口，覆盖：

- 加载、运行、观察和评测场景的通用操作流程；
- 10 个标杆场景各自的场景文件、目标、截止 tick、操作步骤与关键处置；
- 每个场景配套的 Mermaid 时序图与左右布局流程图，标注 `SimulationSourcePanel`、`SimulationProgress`、`SimulationWorldView`、`SimulationActivityFeed`、`SimulationEvaluation` 等实际桌面组件；
- 完成一次场景运行后的结果检查清单。

> 系统架构、通信协议、Hermes ACP 集成原理、录制回放内部机制、原生验收清单等开发/运维向内容不在本文档范围内；如需了解这些内容，请查阅相应源码目录（`crates/cockpit-simulator`、`crates/cockpit-agent`）及内部注释。
> 最新 Desktop Live 只使用真实 `iota-core-acp` 模型路径；`RuleAgent` 仅保留于 CLI、Simulator 合约和测试，不是 Desktop fallback。

## 专题文档

### [Desktop 分层架构与 Sidecar](./architecture-zh.md)

用分层架构图和一次 Live 运行时序图说明：

- Tauri Desktop、React WebView、Simulator 与 Evaluator 的职责边界；
- `sidecar` 是什么、为何需要两个独立进程；
- Hermes Desktop Agent、公开场景、私有 rubric 和 Recording 的数据流；
- 开发模式与打包安装包中 sidecar 的启动差异。
- Windows 上 Hermes ACP CLI、`COCKPIT_HERMES_BIN` 与常见 Live 后端错误的配置方式。
- Live 运行的超时与后端预热不变量：分层 IPC 读超时、每人物计时前预热、以及 Live 回合使用 ephemeral 引擎绕开执行去重台账——附排查顺序与 iota-core 依赖边界说明。

### [独立评测与发布门禁](./evaluation-zh.md)

面向开发者说明：

- 单次 Recording 与 10 场景批量 suite 的 evaluator CLI；
- JSON/JUnit、最低通过率、基线回归和单 case 基础设施错误聚合；
- Desktop evaluator sidecar、私有 rubric、脱敏输入、durable Recording 校验与历史报告隔离；
- CI 工作目录、published `iota-sympantos-core` 依赖和双 sidecar 打包契约。

### [NPC 世界建模](./npc.md)

> 更底层的 Simulator IPC、ACP 适配和 Recording 数据结构仍以 `crates/cockpit-simulator`、`crates/cockpit-agent`、`crates/cockpit-recording` 源码与模块注释为准。

**最后更新时间**：2026-07-18
**统一文档版本**：5.1.0（新增独立评测、批量门禁与 Desktop 报告一致性专题）
