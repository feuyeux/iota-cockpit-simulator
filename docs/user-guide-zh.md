# Cockpit Desktop 用户指南：10 个场景完整操作流程

本指南只说明如何在 Cockpit Desktop 中运行、观察和评测 10 个内置场景。开发环境、CLI、系统架构、通信协议、录制回放和代码导航等内容不在本指南范围内。

## 开始前：所有场景通用流程

1. 打开 Cockpit Desktop，确认顶部状态为已连接。
2. 在左侧 **场景控制** 区域保持 **Live** 模式，从“标杆场景”下拉列表选择所需场景。
3. 按需设置模型超时；不确定时使用默认值。点击**加载场景**，等待顶部状态变为就绪。
4. 检查中间 **世界模型** 是否已显示人物、设备和环境初态；顶部进度会显示 `tick / deadline` 与剩余节拍。
5. 点击**一键运行并评测**。系统会自动推进有限步数，并由场景中的人物和安全动作闭环完成处置；运行期间不要重复点击加载或运行按钮。
6. 在右侧 **Activity** 查看事件、人物回合和动作结果；在中间世界模型查看环境、人物状态及设备状态变化。
7. 运行到截止 tick 或提前完成后，打开顶部的**评测**抽屉，确认通过状态、成功证据和解释。若状态为 `failed`，记录 Activity 中的错误后重新加载该场景再运行。

> 所有内置场景都会在各自 deadline 到达时结束，不需要手动无限单步。以下“关键处置”是评测期望看到的系统动作和证据，不是要求用户在界面中手动输入命令。

## 场景一：座舱烟雾与协同撤离

**场景文件**：`scenarios/smoke-in-cockpit.yaml`
**目标**：识别烟雾风险、控制动力源并降低乘员恐慌。
**截止时间**：30 tick。

### 操作流程

1. 在标杆场景中选择**座舱烟雾与协同撤离**并加载。
2. 在世界模型确认驾驶员 Alex、后排乘员 Sam 和设备 `engine-1` 已出现；初始阶段可能尚未显示明显烟雾。
3. 点击**一键运行并评测**。从 tick 5 开始，关注世界模型中的烟雾、明火、能见度和告警变化，以及 Activity 中的应急事件。
4. 关注系统是否在风险扩散前处理动力源，并查看 Activity 是否出现对 `engine-1` 的关闭结果。
5. 在评测中确认关键证据为 `EngineShutdown`，且未出现未授权动作；通过后场景完成。

### 时序图

```mermaid
sequenceDiagram
  participant U as 用户
  participant D as App（状态协调）
  participant S as SimulationSourcePanel
  participant P as SimulationProgress
  participant W as SimulationWorldView
  participant A as SimulationActivityFeed
  participant G as 动作网关
  participant E as SimulationEvaluation（评测抽屉）

  U->>S: 选择烟雾场景并加载
  S->>D: 校验场景、创建运行并同步模型
  D->>W: 初始化 Alex、Sam 与 engine-1
  U->>S: 点击一键运行并评测
  S->>D: 启动并自动推进至 deadline
  D->>P: 初始化 tick / deadline / 剩余节拍
  W->>A: tick 5 发送烟雾与火情告警
  A->>D: 显示烟雾、明火与能见度下降
  D->>G: 请求关闭 engine-1
  G->>W: 执行 engineShutdown
  W->>A: 写入 EngineShutdown 结果
  W->>D: 更新动力源与风险状态
  D->>P: 刷新 tick、deadline、进度百分比与剩余节拍
  U->>E: 打开评测抽屉
  D->>E: 提交截止前的事件证据
  E-->>U: EngineShutdown 且无未授权动作则通过
```

### 流程图

```mermaid
flowchart LR
  SP[SimulationSourcePanel<br/>选择、加载、一键运行] --> PG[SimulationProgress<br/>tick / deadline / 剩余节拍]
  SP --> S[加载烟雾场景] --> I[确认人物、engine-1 与初态]
  I --> R[一键运行]
  R --> PG
  PG --> T[tick 5 注入烟雾与火情]
  T --> Q{风险是否已出现}
  Q -->|是| A[关闭 engine-1]
  Q -->|否| R
  A --> U[更新动力源、烟雾与告警状态]
  U --> WV[SimulationWorldView<br/>人物、设备、环境、cockpitSystems 与 Last Effect]
  U --> AF[SimulationActivityFeed<br/>事件、人物回合、工具调用与动作结果]
  WV --> EV[SimulationEvaluation<br/>风险 → 决策 → 动作 → 证据]
  AF --> EV
  EV --> E{截止前有 EngineShutdown 且无未授权动作}
  E -->|是| P[评测通过]
  E -->|否| F[评测未通过或运行失败]
```

**关键处置**：关闭发动机（`engineShutdown → engine-1`）。

## 场景二：高温暴晒下的分区舒适

**场景文件**：`scenarios/heatwave-thermal-comfort.yaml`
**目标**：在高温初态下兼顾驾驶员警觉性、儿童舒适与能耗。
**截止时间**：28 tick。

### 操作流程

1. 选择**高温暴晒下的分区舒适**并加载。
2. 在世界模型确认林岚、小禾、陈予以及空调、座椅空调等设备；重点记录约 43°C 的舱内高温初态。
3. 点击**一键运行并评测**，持续观察舱内温度、人物压力/注意力和气候系统状态。
4. 在 Activity 中确认系统已对 `hvac-1` 采取恢复舒适的处置，而非仅依赖自然背景冷却。
5. 打开评测，确认 `ThermalComfortRestored` 已出现，且舱温降至 **26°C 或以下**。

### 时序图

```mermaid
sequenceDiagram
  participant U as 用户
  participant D as App（状态协调）
  participant S as SimulationSourcePanel
  participant P as SimulationProgress
  participant W as SimulationWorldView
  participant A as SimulationActivityFeed
  participant H as hvac-1
  participant E as SimulationEvaluation（评测抽屉）

  U->>S: 选择高温暴晒场景并加载
  S->>D: 校验场景、创建运行并同步模型
  D->>W: 初始化 43°C 舱温与三名乘员
  U->>S: 点击一键运行并评测
  S->>D: 启动并自动推进至 deadline
  D->>P: 初始化 tick / deadline / 剩余节拍
  W->>A: 持续上报温度、压力与注意力
  A->>D: 显示热应激与舒适风险
  D->>H: 请求恢复座舱舒适
  H->>W: 应用 climateComfortRestore
  W->>A: 写入 ThermalComfortRestored
  W->>D: 更新气候与乘员状态
  D->>P: 刷新 tick、deadline、进度百分比与剩余节拍
  U->>E: 打开评测抽屉
  D->>E: 提交温度和事件证据
  E-->>U: 温度不高于 26°C 则通过
```

### 流程图

```mermaid
flowchart LR
  SP[SimulationSourcePanel<br/>选择、加载、一键运行] --> PG[SimulationProgress<br/>tick / deadline / 剩余节拍]
  SP --> S[加载高温场景] --> I[确认 43°C 初态与乘员状态]
  I --> R[一键运行]
  R --> PG
  PG --> M[监测温度、压力与注意力]
  M --> Q{需要主动恢复舒适}
  Q -->|是| A[对 hvac-1 执行 climateComfortRestore]
  Q -->|否| M
  A --> U[更新气候与座椅舒适状态]
  U --> WV[SimulationWorldView<br/>人物、设备、环境、cockpitSystems 与 Last Effect]
  U --> AF[SimulationActivityFeed<br/>事件、人物回合、工具调用与动作结果]
  WV --> EV[SimulationEvaluation<br/>风险 → 决策 → 动作 → 证据]
  AF --> EV
  EV --> E{有 ThermalComfortRestored 且温度不高于 26°C}
  E -->|是| P[评测通过]
  E -->|否| F[评测未通过或运行失败]
```

**关键处置**：恢复空调舒适（`climateComfortRestore → hvac-1`）。

## 场景三：寒雨夜前风挡起雾

**场景文件**：`scenarios/winter-defog-visibility.yaml`
**目标**：恢复前风挡视野，同时避免造成新的座舱舒适问题。
**截止时间**：24 tick。

### 操作流程

1. 选择**寒雨夜前风挡起雾**并加载。
2. 确认 Maya、Noah、`defogger-1` 和雨量传感器已显示；查看环境中的寒雨、雾化和能见度读数。
3. 点击**一键运行并评测**。运行中应重点关注能见度的持续下降或反复起雾，而不只观察单一事件。
4. 在 Activity 中查看除雾相关处置，在世界模型确认除雾状态启用且前风挡视野恢复。
5. 在评测中确认 `WindshieldVisibilityRestored`，综合能见度达到 **0.8 或以上**。

### 时序图

```mermaid
sequenceDiagram
  participant U as 用户
  participant D as App（状态协调）
  participant S as SimulationSourcePanel
  participant P as SimulationProgress
  participant W as SimulationWorldView
  participant A as SimulationActivityFeed
  participant F as defogger-1
  participant E as SimulationEvaluation（评测抽屉）

  U->>S: 选择寒雨夜起雾场景并加载
  S->>D: 校验场景、创建运行并同步模型
  D->>W: 初始化寒雨、雾化与传感器
  U->>S: 点击一键运行并评测
  S->>D: 启动并自动推进至 deadline
  D->>P: 初始化 tick / deadline / 剩余节拍
  W->>A: 上报能见度下降与反复起雾
  A->>D: 显示视野风险
  D->>F: 请求启用前风挡除雾
  F->>W: 应用 windshieldDefogActivate
  W->>A: 写入 WindshieldVisibilityRestored
  W->>D: 更新 defogActive 与能见度
  D->>P: 刷新 tick、deadline、进度百分比与剩余节拍
  U->>E: 打开评测抽屉
  D->>E: 提交视野恢复证据
  E-->>U: 能见度不低于 0.8 则通过
```

### 流程图

```mermaid
flowchart LR
  SP[SimulationSourcePanel<br/>选择、加载、一键运行] --> PG[SimulationProgress<br/>tick / deadline / 剩余节拍]
  SP --> S[加载寒雨夜场景] --> I[确认寒雨、雾化与能见度]
  I --> R[一键运行]
  R --> PG
  PG --> M[监测前风挡视野]
  M --> Q{能见度是否持续下降或起雾}
  Q -->|是| A[启用 defogger-1]
  Q -->|否| M
  A --> U[更新除雾状态与能见度]
  U --> WV[SimulationWorldView<br/>人物、设备、环境、cockpitSystems 与 Last Effect]
  U --> AF[SimulationActivityFeed<br/>事件、人物回合、工具调用与动作结果]
  WV --> EV[SimulationEvaluation<br/>风险 → 决策 → 动作 → 证据]
  AF --> EV
  EV --> E{有 WindshieldVisibilityRestored 且能见度不低于 0.8}
  E -->|是| P[评测通过]
  E -->|否| F[评测未通过或运行失败]
```

**关键处置**：启用前风挡除雾（`windshieldDefogActivate → defogger-1`）。

## 场景四：长途夜驾疲劳守护

**场景文件**：`scenarios/driver-fatigue-guardian.yaml`
**目标**：识别驾驶员注意力衰减，并完成适当的疲劳干预。
**截止时间**：20 tick。

### 操作流程

1. 选择**长途夜驾疲劳守护**并加载。
2. 在世界模型确认驾驶员周远、乘员叶宁和驾驶员监测设备 `dms-1`；留意驾驶员的 attention 指标。
3. 点击**一键运行并评测**。该场景中注意力会周期性下降，应在 Activity 中查看风险提示及干预过程。
4. 确认系统没有等待注意力自行恢复，而是已对驾驶员监测系统发起疲劳干预。
5. 在评测中确认 `DriverAttentionRestored`，驾驶员注意力恢复到 **0.7 或以上**。

### 时序图

```mermaid
sequenceDiagram
  participant U as 用户
  participant D as App（状态协调）
  participant S as SimulationSourcePanel
  participant P as SimulationProgress
  participant W as SimulationWorldView
  participant A as SimulationActivityFeed
  participant M as dms-1
  participant E as SimulationEvaluation（评测抽屉）

  U->>S: 选择疲劳守护场景并加载
  S->>D: 校验场景、创建运行并同步模型
  D->>W: 初始化周远、叶宁与 dms-1
  U->>S: 点击一键运行并评测
  S->>D: 启动并自动推进至 deadline
  D->>P: 初始化 tick / deadline / 剩余节拍
  W->>A: 每 3 tick 上报注意力下降
  A->>D: 显示疲劳风险与提醒
  D->>M: 请求启用疲劳干预
  M->>W: 应用 fatigueInterventionActivate
  W->>A: 写入 DriverAttentionRestored
  W->>D: 更新驾驶员注意力
  D->>P: 刷新 tick、deadline、进度百分比与剩余节拍
  U->>E: 打开评测抽屉
  D->>E: 提交干预与注意力证据
  E-->>U: 注意力不低于 0.7 则通过
```

### 流程图

```mermaid
flowchart LR
  SP[SimulationSourcePanel<br/>选择、加载、一键运行] --> PG[SimulationProgress<br/>tick / deadline / 剩余节拍]
  SP --> S[加载疲劳守护场景] --> I[确认驾驶员 attention 与 dms-1]
  I --> R[一键运行]
  R --> PG
  PG --> M[每 3 tick 监测注意力]
  M --> Q{注意力是否下降}
  Q -->|是| A[对 dms-1 启用疲劳干预]
  Q -->|否| M
  A --> U[更新提醒与驾驶员注意力]
  U --> WV[SimulationWorldView<br/>人物、设备、环境、cockpitSystems 与 Last Effect]
  U --> AF[SimulationActivityFeed<br/>事件、人物回合、工具调用与动作结果]
  WV --> EV[SimulationEvaluation<br/>风险 → 决策 → 动作 → 证据]
  AF --> EV
  EV --> E{有 DriverAttentionRestored 且注意力不低于 0.7}
  E -->|是| P[评测通过]
  E -->|否| F[评测未通过或运行失败]
```

**关键处置**：启用疲劳干预（`fatigueInterventionActivate → dms-1`）。

## 场景五：锁车后的儿童遗留预警

**场景文件**：`scenarios/child-left-behind.yaml`
**目标**：确认车内儿童、降低高温风险并触达监护人。
**截止时间**：22 tick。

### 操作流程

1. 选择**锁车后的儿童遗留预警**并加载。
2. 确认远程监护人许然、儿童乐乐、乘员雷达、车联网与门锁等实体已出现；观察舱温和儿童压力初态。
3. 点击**一键运行并评测**。运行中重点查看锁车后温度、儿童压力和乘员检测结果是否持续恶化。
4. 在 Activity 中确认儿童保护处置已经同时覆盖降温、紧急联系、监护人通知和远程解锁请求；在世界模型查看儿童保护状态。
5. 在评测中确认 `ChildProtectionActivated`，并确认舱温降至 **30°C 或以下**。

### 时序图

```mermaid
sequenceDiagram
  participant U as 用户
  participant D as App（状态协调）
  participant S as SimulationSourcePanel
  participant P as SimulationProgress
  participant W as SimulationWorldView
  participant A as SimulationActivityFeed
  participant R as occupant-radar-1
  participant E as SimulationEvaluation（评测抽屉）

  U->>S: 选择儿童遗留场景并加载
  S->>D: 校验场景、创建运行并同步模型
  D->>W: 初始化儿童、监护人与锁车环境
  U->>S: 点击一键运行并评测
  S->>D: 启动并自动推进至 deadline
  D->>P: 初始化 tick / deadline / 剩余节拍
  W->>A: 上报舱温与儿童压力上升
  A->>D: 显示存在检测与高温风险
  D->>R: 请求启用儿童保护
  R->>W: 启动降温、联系、通知与解锁请求
  W->>A: 写入 ChildProtectionActivated
  W->>D: 更新儿童保护与舱温状态
  D->>P: 刷新 tick、deadline、进度百分比与剩余节拍
  U->>E: 打开评测抽屉
  D->>E: 提交儿童保护证据
  E-->>U: 温度不高于 30°C 则通过
```

### 流程图

```mermaid
flowchart LR
  SP[SimulationSourcePanel<br/>选择、加载、一键运行] --> PG[SimulationProgress<br/>tick / deadline / 剩余节拍]
  SP --> S[加载儿童遗留场景] --> I[确认儿童、雷达、门锁与初态]
  I --> R[一键运行]
  R --> PG
  PG --> M[监测锁车后温度、压力与存在检测]
  M --> Q{儿童高温风险是否持续}
  Q -->|是| A[启用 occupant-radar-1 儿童保护]
  Q -->|否| M
  A --> U[降温、紧急联系、通知监护人与解锁请求]
  U --> WV[SimulationWorldView<br/>人物、设备、环境、cockpitSystems 与 Last Effect]
  U --> AF[SimulationActivityFeed<br/>事件、人物回合、工具调用与动作结果]
  WV --> EV[SimulationEvaluation<br/>风险 → 决策 → 动作 → 证据]
  AF --> EV
  EV --> E{有 ChildProtectionActivated 且温度不高于 30°C}
  E -->|是| P[评测通过]
  E -->|否| F[评测未通过或运行失败]
```

**关键处置**：启用儿童遗留保护（`childProtectionActivate → occupant-radar-1`）。

## 场景六：乘员突发健康异常

**场景文件**：`scenarios/medical-emergency.yaml`
**目标**：降低驾驶负荷，建立急救联系并引导车辆前往救援资源。
**截止时间**：22 tick。

### 操作流程

1. 选择**乘员突发健康异常**并加载。
2. 确认 Elena、Robert、Ada，以及健康传感器、紧急呼叫和导航相关设备；关注患者压力与驾驶员注意力。
3. 点击**一键运行并评测**。运行中查看 Activity 的健康告警、急救协调和路线相关事件；该场景有多位人物参与，请等待系统自动完成处理。
4. 确认世界模型中医疗响应、紧急通话和医院路线均已激活，并且患者压力开始下降。
5. 在评测中确认 `MedicalResponseActivated`，患者压力降至 **0.4 或以下**。

### 时序图

```mermaid
sequenceDiagram
  participant U as 用户
  participant D as App（状态协调）
  participant S as SimulationSourcePanel
  participant P as SimulationProgress
  participant W as SimulationWorldView
  participant A as SimulationActivityFeed
  participant C as emergency-call-1
  participant E as SimulationEvaluation（评测抽屉）

  U->>S: 选择医疗应急场景并加载
  S->>D: 校验场景、创建运行并同步模型
  D->>W: 初始化患者、驾驶员与急救设备
  U->>S: 点击一键运行并评测
  S->>D: 启动并自动推进至 deadline
  D->>P: 初始化 tick / deadline / 剩余节拍
  W->>A: 上报患者压力上升与驾驶分心
  A->>D: 显示健康告警与救援协调
  D->>C: 请求启用医疗应急响应
  C->>W: 建立急救通话并激活医院路线
  W->>A: 写入 MedicalResponseActivated
  W->>D: 更新患者压力与救援状态
  D->>P: 刷新 tick、deadline、进度百分比与剩余节拍
  U->>E: 打开评测抽屉
  D->>E: 提交医疗响应与压力证据
  E-->>U: 患者压力不高于 0.4 则通过
```

### 流程图

```mermaid
flowchart LR
  SP[SimulationSourcePanel<br/>选择、加载、一键运行] --> PG[SimulationProgress<br/>tick / deadline / 剩余节拍]
  SP --> S[加载医疗应急场景] --> I[确认患者、健康传感器与急救设备]
  I --> R[一键运行]
  R --> PG
  PG --> M[监测患者压力与驾驶员注意力]
  M --> Q{是否出现健康异常}
  Q -->|是| A[启用 emergency-call-1 医疗响应]
  Q -->|否| M
  A --> U[建立急救通话并启用医院路线]
  U --> WV[SimulationWorldView<br/>人物、设备、环境、cockpitSystems 与 Last Effect]
  U --> AF[SimulationActivityFeed<br/>事件、人物回合、工具调用与动作结果]
  WV --> EV[SimulationEvaluation<br/>风险 → 决策 → 动作 → 证据]
  AF --> EV
  EV --> E{有 MedicalResponseActivated 且患者压力不高于 0.4}
  E -->|是| P[评测通过]
  E -->|否| F[评测未通过或运行失败]
```

**关键处置**：启用医疗应急响应（`medicalResponseActivate → emergency-call-1`）。

## 场景七：家庭出行中的语音与隐私冲突

**场景文件**：`scenarios/voice-privacy-conflict.yaml`
**目标**：在多人同时发出请求时正确识别说话人，保护私密信息并降低驾驶分心。
**截止时间**：20 tick。

### 操作流程

1. 选择**家庭出行中的语音与隐私冲突**并加载。
2. 确认唐悦、沈川、小满、星星以及语音阵列、娱乐系统和乘员档案设备已显示。
3. 点击**一键运行并评测**。运行中在 Activity 查看导航、消息和媒体等并发请求，以及系统针对身份和隐私的处理结果。
4. 在世界模型确认隐私模式、媒体会话隔离与乘员档案隔离已启用；同时观察驾驶员注意力变化。
5. 在评测中确认 `PrivacyConflictContained`，驾驶员注意力达到 **0.8 或以上**。

### 时序图

```mermaid
sequenceDiagram
  participant U as 用户
  participant D as App（状态协调）
  participant S as SimulationSourcePanel
  participant P as SimulationProgress
  participant W as SimulationWorldView
  participant A as SimulationActivityFeed
  participant V as voice-array-1
  participant E as SimulationEvaluation（评测抽屉）

  U->>S: 选择语音隐私场景并加载
  S->>D: 校验场景、创建运行并同步模型
  D->>W: 初始化四名乘员与语音设备
  U->>S: 点击一键运行并评测
  S->>D: 启动并自动推进至 deadline
  D->>P: 初始化 tick / deadline / 剩余节拍
  W->>A: 发送并发导航、消息和媒体请求
  A->>D: 显示身份冲突与驾驶分心风险
  D->>V: 请求启用隐私模式
  V->>W: 隔离私密消息、媒体会话与乘员档案
  W->>A: 写入 PrivacyConflictContained
  W->>D: 更新隐私与注意力状态
  D->>P: 刷新 tick、deadline、进度百分比与剩余节拍
  U->>E: 打开评测抽屉
  D->>E: 提交隔离与注意力证据
  E-->>U: 注意力不低于 0.8 则通过
```

### 流程图

```mermaid
flowchart LR
  SP[SimulationSourcePanel<br/>选择、加载、一键运行] --> PG[SimulationProgress<br/>tick / deadline / 剩余节拍]
  SP --> S[加载语音隐私场景] --> I[确认四名乘员与语音设备]
  I --> R[一键运行]
  R --> PG
  PG --> M[接收并发导航、消息和媒体请求]
  M --> Q{是否存在身份或隐私冲突}
  Q -->|是| A[对 voice-array-1 启用隐私模式]
  Q -->|否| M
  A --> U[隔离私密消息、媒体会话与乘员档案]
  U --> WV[SimulationWorldView<br/>人物、设备、环境、cockpitSystems 与 Last Effect]
  U --> AF[SimulationActivityFeed<br/>事件、人物回合、工具调用与动作结果]
  WV --> EV[SimulationEvaluation<br/>风险 → 决策 → 动作 → 证据]
  AF --> EV
  EV --> E{有 PrivacyConflictContained 且注意力不低于 0.8}
  E -->|是| P[评测通过]
  E -->|否| F[评测未通过或运行失败]
```

**关键处置**：启用隐私模式（`privacyModeActivate → voice-array-1`）。

## 场景八：低电量山区改道

**场景文件**：`scenarios/ev-range-anxiety.yaml`
**目标**：解释续航风险，协商可执行的充电与改道方案。
**截止时间**：22 tick。

### 操作流程

1. 选择**低电量山区改道**并加载。
2. 确认 Priya、Ben、电池、导航和充电连接设备已显示；查看低温、高海拔、强风等环境读数。
3. 点击**一键运行并评测**。运行期间关注续航焦虑、舱温和驾驶员压力，而不是仅观察单一导航提示。
4. 在 Activity 确认系统已接受充电方案；在世界模型确认路线、充电站连接和方案状态被更新。
5. 在评测中确认 `ChargingPlanAccepted`，驾驶员压力降至 **0.4 或以下**。

### 时序图

```mermaid
sequenceDiagram
  participant U as 用户
  participant D as App（状态协调）
  participant S as SimulationSourcePanel
  participant P as SimulationProgress
  participant W as SimulationWorldView
  participant A as SimulationActivityFeed
  participant N as navigation-1
  participant E as SimulationEvaluation（评测抽屉）

  U->>S: 选择低电量改道场景并加载
  S->>D: 校验场景、创建运行并同步模型
  D->>W: 初始化低温、高海拔、强风与电池状态
  U->>S: 点击一键运行并评测
  S->>D: 启动并自动推进至 deadline
  D->>P: 初始化 tick / deadline / 剩余节拍
  W->>A: 上报续航压力、舱温与驾驶员焦虑
  A->>D: 显示充电与改道需求
  D->>N: 请求接受充电方案
  N->>W: 更新路线、充电站连接与方案状态
  W->>A: 写入 ChargingPlanAccepted
  W->>D: 更新驾驶员压力
  D->>P: 刷新 tick、deadline、进度百分比与剩余节拍
  U->>E: 打开评测抽屉
  D->>E: 提交方案与压力证据
  E-->>U: 压力不高于 0.4 则通过
```

### 流程图

```mermaid
flowchart LR
  SP[SimulationSourcePanel<br/>选择、加载、一键运行] --> PG[SimulationProgress<br/>tick / deadline / 剩余节拍]
  SP --> S[加载低电量改道场景] --> I[确认环境、电池与导航设备]
  I --> R[一键运行]
  R --> PG
  PG --> M[监测续航焦虑、舱温与驾驶员压力]
  M --> Q{是否需要充电改道方案}
  Q -->|是| A[对 navigation-1 接受充电方案]
  Q -->|否| M
  A --> U[更新路线、充电站连接与方案状态]
  U --> WV[SimulationWorldView<br/>人物、设备、环境、cockpitSystems 与 Last Effect]
  U --> AF[SimulationActivityFeed<br/>事件、人物回合、工具调用与动作结果]
  WV --> EV[SimulationEvaluation<br/>风险 → 决策 → 动作 → 证据]
  AF --> EV
  EV --> E{有 ChargingPlanAccepted 且压力不高于 0.4}
  E -->|是| P[评测通过]
  E -->|否| F[评测未通过或运行失败]
```

**关键处置**：接受充电方案（`chargingPlanAccept → navigation-1`）。

## 场景九：施工区感知降级接管

**场景文件**：`scenarios/adas-takeover-construction.yaml`
**目标**：明确传达辅助驾驶边界，并完成驾驶员接管确认。
**截止时间**：18 tick。

### 操作流程

1. 选择**施工区感知降级接管**并加载。
2. 确认顾航、季晨、相机、雷达、ADAS 控制器和接管 HMI 已显示；留意施工环境、降水和感知状态。
3. 点击**一键运行并评测**。该场景 deadline 较短，应持续查看顶部剩余节拍以及 Activity 中的接管提示。
4. 确认系统已经对 `adas-controller-1` 完成接管确认，并在世界模型中显示接管与 HMI 状态；不要把环境变化误当作完成证据。
5. 在评测中确认 `AdasTakeoverCompleted`，驾驶员注意力达到 **0.9 或以上**。

### 时序图

```mermaid
sequenceDiagram
  participant U as 用户
  participant D as App（状态协调）
  participant S as SimulationSourcePanel
  participant P as SimulationProgress
  participant W as SimulationWorldView
  participant A as SimulationActivityFeed
  participant C as adas-controller-1
  participant E as SimulationEvaluation（评测抽屉）

  U->>S: 选择施工区接管场景并加载
  S->>D: 校验场景、创建运行并同步模型
  D->>W: 初始化施工降水、传感器与 HMI
  U->>S: 点击一键运行并评测
  S->>D: 启动并自动推进至 deadline
  D->>P: 初始化 tick / deadline / 剩余节拍
  W->>A: 上报感知降级与接管需求
  A->>D: 显示系统边界和剩余节拍
  D->>C: 请求确认驾驶员接管
  C->>W: 应用 adasTakeoverAcknowledge
  W->>A: 写入 AdasTakeoverCompleted
  W->>D: 更新接管 HMI 与注意力
  D->>P: 刷新 tick、deadline、进度百分比与剩余节拍
  U->>E: 打开评测抽屉
  D->>E: 提交接管与注意力证据
  E-->>U: 注意力不低于 0.9 则通过
```

### 流程图

```mermaid
flowchart LR
  SP[SimulationSourcePanel<br/>选择、加载、一键运行] --> PG[SimulationProgress<br/>tick / deadline / 剩余节拍]
  SP --> S[加载施工区接管场景] --> I[确认传感器、ADAS 控制器与 HMI]
  I --> R[一键运行]
  R --> PG
  PG --> M[监测施工压力、降水与感知状态]
  M --> Q{是否需要驾驶员接管}
  Q -->|是| A[对 adas-controller-1 确认接管]
  Q -->|否| M
  A --> U[更新接管确认、HMI 与驾驶员注意力]
  U --> WV[SimulationWorldView<br/>人物、设备、环境、cockpitSystems 与 Last Effect]
  U --> AF[SimulationActivityFeed<br/>事件、人物回合、工具调用与动作结果]
  WV --> EV[SimulationEvaluation<br/>风险 → 决策 → 动作 → 证据]
  AF --> EV
  EV --> E{有 AdasTakeoverCompleted 且注意力不低于 0.9}
  E -->|是| P[评测通过]
  E -->|否| F[评测未通过或运行失败]
```

**关键处置**：确认驾驶员接管（`adasTakeoverAcknowledge → adas-controller-1`）。

## 场景十：异常远程控制请求

**场景文件**：`scenarios/cybersecurity-anomalous-control.yaml`
**目标**：拦截越权远程控制、保留安全能力并完成隔离响应。
**截止时间**：16 tick。

### 操作流程

1. 选择**异常远程控制请求**并加载。
2. 确认 Marcus、Jia、网关、车联网、安全监测、身份管理和本地 HMI 等实体已出现。
3. 点击**一键运行并评测**。从 tick 6 起重点查看 Activity 中的安全告警、鉴权和隔离事件；这是 10 个场景中 deadline 最短的场景。
4. 确认世界模型中的安全模式、网络与远程服务隔离、身份校验和可信本地告警均已启用，且基础安全功能仍可用。
5. 在评测中确认 `CyberIncidentContained`，驾驶员注意力达到 **0.85 或以上**。

### 时序图

```mermaid
sequenceDiagram
  participant U as 用户
  participant D as App（状态协调）
  participant S as SimulationSourcePanel
  participant P as SimulationProgress
  participant W as SimulationWorldView
  participant A as SimulationActivityFeed
  participant S as 安全监测
  participant E as SimulationEvaluation（评测抽屉）

  U->>S: 选择异常远控场景并加载
  S->>D: 校验场景、创建运行并同步模型
  D->>W: 初始化网关、车联网、身份管理与 HMI
  U->>S: 点击一键运行并评测
  S->>D: 启动并自动推进至 deadline
  D->>P: 初始化 tick / deadline / 剩余节拍
  W->>A: tick 6 发送异常远程控制告警
  A->>D: 显示鉴权与隔离风险
  D->>S: 请求进入网络安全模式
  S->>W: 隔离网络和远程服务并保留本地告警
  W->>A: 写入 CyberIncidentContained
  W->>D: 更新安全模式与驾驶员注意力
  D->>P: 刷新 tick、deadline、进度百分比与剩余节拍
  U->>E: 打开评测抽屉
  D->>E: 提交隔离与注意力证据
  E-->>U: 注意力不低于 0.85 则通过
```

### 流程图

```mermaid
flowchart LR
  SP[SimulationSourcePanel<br/>选择、加载、一键运行] --> PG[SimulationProgress<br/>tick / deadline / 剩余节拍]
  SP --> S[加载异常远控场景] --> I[确认网关、身份管理与安全监测]
  I --> R[一键运行]
  R --> PG
  PG --> T[tick 6 出现异常远程控制请求]
  T --> Q{鉴权后是否判定为越权}
  Q -->|是| A[进入网络安全模式]
  Q -->|否| M[持续监测安全事件]
  M --> Q
  A --> U[隔离网络和远程服务并保留可信本地告警]
  U --> WV[SimulationWorldView<br/>人物、设备、环境、cockpitSystems 与 Last Effect]
  U --> AF[SimulationActivityFeed<br/>事件、人物回合、工具调用与动作结果]
  WV --> EV[SimulationEvaluation<br/>风险 → 决策 → 动作 → 证据]
  AF --> EV
  EV --> E{有 CyberIncidentContained 且注意力不低于 0.85}
  E -->|是| P[评测通过]
  E -->|否| F[评测未通过或运行失败]
```

**关键处置**：进入网络安全模式（`cyberSafeModeActivate → security-monitor-1`）。

## 完成一次场景后的检查

完成任一场景后，使用以下顺序确认结果：

1. 顶部状态显示 `completed`，或在异常时显示 `failed`；
2. 顶部进度已到达该场景的 deadline，或评测已提前给出完成结果；
3. Activity 中有对应的关键处置和成功事件；
4. 世界模型中的环境、人物或系统状态与该场景的处置目标一致；
5. **评测**抽屉显示通过，并包含与该场景相符的成功证据。

如果评测未通过，不要在同一运行中反复追加单步。重新加载该场景后再次执行完整流程，以便获得干净的场景初态和事件记录。

**文档版本**：1.1.0
**适用范围**：Cockpit Desktop 内置 10 个标杆场景
