# Cockpit 独立评测与发布门禁

本文面向维护 Cockpit Simulator 的开发者，说明独立 evaluator、批量套件、Desktop 报告和 CI 发布门禁的当前实现。场景操作步骤仍以[用户指南](./user-guide-zh.md)为准。

## 1. 边界与数据流

独立评测位于模拟进程之外：

```text
cockpit-simulator / RecordingStore
  -> immutable, redacted Recording
  -> cockpit-evaluator + private rubric
  -> EvidenceVerdict
  -> JSON / JUnit / Desktop history
```

边界约束如下：

- `cockpit-simulator` 只执行模拟并产出 Recording，不读取私有 rubric，也不决定发布门禁；
- `cockpit-evaluator` 读取不可变 Recording，在独立进程中执行确定性规则，并可选调用两个隔离 Judge provider；
- 私有 rubric 位于 `evaluations/private/`，不会传给执行模型；Desktop 打包时作为 Tauri resource 放入原生层；
- Recording 交给 evaluator 前统一经过脱敏序列化，避免把嵌套凭证、prompt、narrative 或 utterance 写入临时输入；
- WebView 只能提交 `runId` 和 `scenarioId`，不能注入 Judge 可执行命令。Judge 配置属于 Tauri 原生信任边界。

## 2. 单次评测

从 Simulator 导出的 JSON Recording 评测：

```bash
target/debug/cockpit-evaluator \
  --recording target/run-recording.json \
  --rubric evaluations/private/smoke-in-cockpit.yaml
```

也可以直接从只读 SQLite RecordingStore 加载指定运行：

```bash
target/debug/cockpit-evaluator \
  --recording-db target/recordings.sqlite \
  --run-id run-smoke-in-cockpit \
  --rubric evaluations/private/smoke-in-cockpit.yaml
```

两种输入方式只能选择一种。单次结果输出为 `EvidenceVerdict` JSON；`releaseGatePassed=false` 或 verdict 为 `fail` 时进程退出码为 `2`。

### 双 Judge

需要 Judge 时，必须同时配置 A/B 两个 provider：

```bash
target/debug/cockpit-evaluator \
  --recording target/run-recording.json \
  --rubric evaluations/private/smoke-in-cockpit.yaml \
  --judge-a-command /path/to/judge-a \
  --judge-b-command /path/to/judge-b \
  --judge-timeout-ms 120000
```

Provider 从 stdin 接收一个 `JudgeRequest` JSON，并在 stdout 返回一个 `JudgeDecision` JSON。结果必须包含 Judge 身份、模型、prompt/rubric/schema hash、0..=1 的 confidence 和 Recording 证据引用。只配置一侧 provider、hash 不匹配、无证据或输出超过限制都会失败。未配置 provider 时只运行确定性评测。

## 3. 批量套件

默认套件 `evaluations/suite.yaml` 覆盖全部 10 个标杆场景。运行命令：

```bash
target/debug/cockpit-evaluator \
  --suite evaluations/suite.yaml \
  --simulator-command target/debug/cockpit-simulator \
  --json-report target/evaluation-report.json \
  --junit-report target/evaluation-junit.xml
```

套件 case 必须定义且只能定义以下一种输入：

- `scenario`：evaluator 调用 `--simulator-command` 生成 Recording；
- `recording`：读取已有 Recording JSON；
- `recordingDb` + `runId`：从 SQLite RecordingStore 加载。

`mode` 可为 `deterministic` 或 `live`；`ticks` 默认 `80`，`timeoutMs` 默认 `2000`。所有相对路径均相对于 suite manifest 所在目录解析，case ID 必须非空且唯一。

### 错误聚合

单个 case 的 rubric、Recording、Simulator 或 evaluator 错误不会提前终止整个 case 循环。该 case 会写入：

- verdict：`inconclusive`；
- `releaseGatePassed=false`；
- `infrastructureError` 和可审计 explanation；
- 空 Judge/证据列表和稳定的 schema hash。

其他 case 继续执行，JSON 与 JUnit 报告仍会生成。suite manifest 无法读取/解析、schema 不支持、case ID 重复或 baseline 文件本身无效属于 suite 级错误，仍会直接返回错误。

### 通过率与基线

```bash
target/debug/cockpit-evaluator \
  --suite evaluations/suite.yaml \
  --simulator-command target/debug/cockpit-simulator \
  --baseline target/previous-evaluation-report.json \
  --minimum-pass-rate 0.9 \
  --json-report target/evaluation-report.json \
  --junit-report target/evaluation-junit.xml
```

一个 case 只有同时满足 `verdict=pass` 和 `releaseGatePassed=true` 才计入通过率。批量门禁要求：

1. 通过率不低于 `--minimum-pass-rate`；
2. 回归数量为 0。

基线中成功、当前不再成功的 case 记为回归。基线中存在但当前 suite 已删除的 case 不会被忽略：报告会补入一个 `inconclusive` case，并设置 `regressed=true`。门禁失败时报告照常写出，进程退出码为 `2`。

JUnit 将每个非成功 case 写为 `<failure>`，内容包含 EvidenceVerdict explanation；基线回归另写入 `<system-err>`。

## 4. Desktop 一键独立评测

评测抽屉中的“一键独立评测”走以下路径：

1. WebView 调用原生 `evaluate_run(runId, scenarioId)`；
2. Tauri 从 Simulator 获取 Recording，并校验 Recording 的 scenario 与请求的私有 rubric 一致；
3. 原生层写入临时脱敏 Recording，启动 `cockpit-evaluator` sidecar；
4. evaluator 返回 EvidenceVerdict 后，Tauri 原子写入应用数据目录下的 `evaluation-history/`；
5. 前端显示 verdict、发布门禁、确定性规则、证据、Judge provenance 和导出入口。

### Process 模式的一致性保护

Embedded 模式直接读取 Simulator 内存中的 Recording。Process 模式读取 SQLite 后，还会通过已鉴权 IPC 调用 `GetSimulationSnapshot`，并比较：

- Recording `runId` 与当前 Simulator snapshot `runId`；
- durable tick 与 snapshot tick。durable tick 等于最后一条 `StepRecord.tick + 1`，空 Recording 为 `0`。

任一项不一致都拒绝评测，避免持久化失败或落后一 tick 时对陈旧 Recording 打分。用户应先处理 Simulator 持久化错误，再重新评测，而不是绕过该校验。

### 历史和跨运行显示

- 历史最多保留最近 100 份报告，单个损坏 JSON 会被跳过，不阻断其他历史；
- 切换活动 run 时，前端只自动选择 `runId` 匹配的报告；没有匹配项时会清除旧选中报告；
- 报告顶部始终显示所属 `scenarioId` 和 `runId`。用户主动查看其他历史时也不会把旧 PASS 误认为当前运行结果。

Desktop Judge provider 只能由原生进程环境配置：

| 环境变量 | 说明 |
| :--- | :--- |
| `COCKPIT_EVALUATOR_BIN` | 开发环境覆盖 evaluator 可执行文件 |
| `COCKPIT_JUDGE_A_BIN` / `COCKPIT_JUDGE_B_BIN` | 必须成对配置的 Judge provider |
| `COCKPIT_JUDGE_A_ARGS_JSON` / `COCKPIT_JUDGE_B_ARGS_JSON` | provider 参数 JSON 数组 |
| `COCKPIT_JUDGE_TIMEOUT_MS` | 每个 provider 的超时毫秒数 |

## 5. CI、依赖与 sidecar 打包

仓库根目录本身就是 Rust workspace。CI 的 Rust job 在根目录运行，并在无 Cargo cache 的干净环境执行 `cargo fetch --locked`，验证锁定的 crates.io 包 `iota-sympantos-core` 可解析，不依赖旧 Git revision。

发布门禁构建 Simulator/evaluator 后执行默认 10 场景 suite，并始终上传：

- `target/evaluation-report.json`；
- `target/evaluation-junit.xml`。

Tauri `externalBin` 同时声明：

- `binaries/cockpit-simulator`；
- `binaries/cockpit-evaluator`。

生产打包脚本会构建并暂存两个 target-specific sidecar。`src-tauri/build.rs` 只在 workspace 测试或尚未暂存真实 binary 时创建对应平台的空占位文件，使 Tauri 配置校验不因第二个 external binary 缺失而提前失败；占位文件不是可发布的 evaluator。

## 6. 关键回归契约

- evaluator suite 单元测试覆盖基础设施失败 case 和缺失基线 case；
- Desktop 前端测试覆盖从有 PASS 的 `run-1` 切换到无匹配历史的 `run-2`，确认旧报告被清除；
- Simulator restart 合约使用普通 persistent run 写入 open-world goal/checkpoint，销毁并重建 `SimulatorHandler` 后验证 SQLite checkpoint 与 `ResumeSimulation`；该测试不启动 live ACP，因此不会因 workspace feature unification 启用真实 Hermes 而不稳定；
- Process Recording 校验测试覆盖 run ID/tick 一致、落后一 tick 和 run ID 不匹配。

## 7. 排障

| 现象 | 检查 |
| :--- | :--- |
| 单个 suite case 为 `inconclusive` | 查看 `infrastructureError`、JUnit `<failure>` 和 Simulator stderr 摘要 |
| suite 直接返回错误且无报告 | 检查 manifest/schema、重复 ID、baseline JSON 和 suite 级路径 |
| Desktop 提示 Recording 不处于当前 durable snapshot | 检查 Simulator 持久化事件，确认 SQLite run ID/tick 已追上当前 snapshot |
| Desktop 找不到 evaluator | 检查 `COCKPIT_EVALUATOR_BIN` 或打包后的 evaluator sidecar |
| Judge 配置失败 | 确认 A/B 同时配置、参数是 JSON 数组、provider 输出单个合法 JSON |
| 新运行仍看到历史报告 | 先看报告顶部 scenario/run 身份；自动选择只会匹配当前 run，历史按钮允许显式查看旧报告 |
