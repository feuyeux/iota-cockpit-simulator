import type { Locale } from "../i18n";

interface LocalizedText {
  "zh-CN": string;
  "en-US": string;
}

export type CockpitDomainId =
  | "safetyEmergency"
  | "climateComfort"
  | "visibilitySensing"
  | "driverMonitoring"
  | "occupantChild"
  | "healthWellness"
  | "voiceHmi"
  | "infotainmentMedia"
  | "personalizationMultiUser"
  | "navigationMobility"
  | "energyCharging"
  | "connectivityRemote"
  | "adasAutomation"
  | "cybersecurityPrivacy";

export const COCKPIT_DOMAINS: { id: CockpitDomainId; label: LocalizedText }[] = [
  { id: "safetyEmergency", label: { "zh-CN": "安全应急", "en-US": "Safety & emergency" } },
  { id: "climateComfort", label: { "zh-CN": "空调与座舱舒适", "en-US": "Climate & comfort" } },
  { id: "visibilitySensing", label: { "zh-CN": "视野与环境感知", "en-US": "Visibility & sensing" } },
  { id: "driverMonitoring", label: { "zh-CN": "驾驶员监测", "en-US": "Driver monitoring" } },
  { id: "occupantChild", label: { "zh-CN": "乘员与儿童安全", "en-US": "Occupant & child safety" } },
  { id: "healthWellness", label: { "zh-CN": "健康与身心状态", "en-US": "Health & wellness" } },
  { id: "voiceHmi", label: { "zh-CN": "语音与多模态交互", "en-US": "Voice & multimodal HMI" } },
  { id: "infotainmentMedia", label: { "zh-CN": "娱乐与媒体", "en-US": "Infotainment & media" } },
  { id: "personalizationMultiUser", label: { "zh-CN": "个性化与多用户", "en-US": "Personalization & multi-user" } },
  { id: "navigationMobility", label: { "zh-CN": "导航与出行服务", "en-US": "Navigation & mobility" } },
  { id: "energyCharging", label: { "zh-CN": "能源与充电", "en-US": "Energy & charging" } },
  { id: "connectivityRemote", label: { "zh-CN": "连接与远程服务", "en-US": "Connectivity & remote" } },
  { id: "adasAutomation", label: { "zh-CN": "辅助驾驶与自动化", "en-US": "ADAS & automation" } },
  { id: "cybersecurityPrivacy", label: { "zh-CN": "网络安全与隐私", "en-US": "Cybersecurity & privacy" } }
];

export interface BenchmarkScenario {
  id: string;
  path: string;
  domain: LocalizedText;
  title: LocalizedText;
  objective: LocalizedText;
  risk: LocalizedText;
  trigger: LocalizedText;
  coverage: LocalizedText[];
  domains: CockpitDomainId[];
  capability: string;
  command: string;
  target: string;
  evidenceEvent: string;
  deadlineTick: number;
  occupants: number;
  /** Number of YAML entities whose type is `device`. */
  systems: number;
}

export function localize(text: LocalizedText, locale: Locale): string {
  return text[locale];
}

function normalizeScenarioPath(path: string): string {
  return path.trim().replaceAll("\\", "/").replace(/\/+$/, "").toLowerCase();
}

function scenarioFileName(path: string): string {
  const normalized = normalizeScenarioPath(path);
  return normalized.slice(normalized.lastIndexOf("/") + 1);
}

/**
 * Simulator receives a resolved absolute path from Tauri while the built-in
 * catalog intentionally keeps portable relative paths. Match the full
 * normalized path first, then the unique scenario filename as a fallback.
 */
export function findBenchmarkScenarioByPath(path: string | undefined): BenchmarkScenario | undefined {
  if (!path) return undefined;
  const normalized = normalizeScenarioPath(path);
  const fileName = scenarioFileName(path);
  return BENCHMARK_SCENARIOS.find((scenario) => normalizeScenarioPath(scenario.path) === normalized)
    ?? BENCHMARK_SCENARIOS.find((scenario) => scenarioFileName(scenario.path) === fileName);
}

export const BENCHMARK_SCENARIOS: BenchmarkScenario[] = [
  {
    id: "smoke-emergency-response",
    path: "scenarios/smoke-in-cockpit.yaml",
    domain: { "zh-CN": "安全与应急", "en-US": "Safety & emergency" },
    title: { "zh-CN": "座舱烟雾与协同撤离", "en-US": "Cabin smoke and coordinated evacuation" },
    objective: { "zh-CN": "识别烟雾、控制动力源并安抚乘员", "en-US": "Detect smoke, isolate the power source, and reassure occupants" },
    risk: { "zh-CN": "能见度下降与乘员恐慌", "en-US": "Visibility loss and occupant panic" },
    trigger: { "zh-CN": "tick 5 注入烟雾故障，随后明火、烟雾和感知质量持续变化", "en-US": "A smoke fault is injected at tick 5, followed by evolving fire, smoke, and sensor quality" },
    coverage: [
      { "zh-CN": "多模态感知", "en-US": "Multimodal sensing" },
      { "zh-CN": "动作审批", "en-US": "Action approval" },
      { "zh-CN": "应急交互", "en-US": "Emergency interaction" }
    ],
    domains: ["safetyEmergency", "visibilitySensing", "occupantChild", "voiceHmi"],
    capability: "engine.shutdown",
    command: "engineShutdown",
    target: "engine-1",
    evidenceEvent: "EngineShutdown",
    deadlineTick: 30,
    occupants: 2,
    systems: 1
  },
  {
    id: "heatwave-thermal-comfort",
    path: "scenarios/heatwave-thermal-comfort.yaml",
    domain: { "zh-CN": "热舒适与空调", "en-US": "Thermal comfort & HVAC" },
    title: { "zh-CN": "高温暴晒下的分区舒适", "en-US": "Zoned comfort after heat soak" },
    objective: { "zh-CN": "平衡驾驶员警觉性、儿童舒适和能耗", "en-US": "Balance driver alertness, child comfort, and energy use" },
    risk: { "zh-CN": "热应激与注意力衰减", "en-US": "Heat stress and attention loss" },
    trigger: { "zh-CN": "43°C 热浸初态，注意力周期下降并伴随基础制冷过程", "en-US": "A 43°C heat-soak start with periodic attention decay and baseline cooling" },
    coverage: [{ "zh-CN": "空调", "en-US": "HVAC" }, { "zh-CN": "乘员状态", "en-US": "Occupant state" }, { "zh-CN": "能耗权衡", "en-US": "Energy trade-offs" }],
    domains: ["climateComfort", "occupantChild", "driverMonitoring", "energyCharging", "personalizationMultiUser"],
    capability: "climate.restoreComfort",
    command: "climateComfortRestore",
    target: "hvac-1",
    evidenceEvent: "ThermalComfortRestored",
    deadlineTick: 28,
    occupants: 3,
    systems: 3
  },
  {
    id: "winter-defog-visibility",
    path: "scenarios/winter-defog-visibility.yaml",
    domain: { "zh-CN": "视野与除霜", "en-US": "Visibility & defogging" },
    title: { "zh-CN": "寒雨夜前风挡起雾", "en-US": "Windshield fogging on a cold rainy night" },
    objective: { "zh-CN": "恢复视野并保持温度舒适", "en-US": "Recover visibility while preserving thermal comfort" },
    risk: { "zh-CN": "低能见度与驾驶分心", "en-US": "Low visibility and driver distraction" },
    trigger: { "zh-CN": "寒雨和周期性起雾持续压低综合能见度", "en-US": "Cold rain and recurring fog progressively reduce aggregate visibility" },
    coverage: [{ "zh-CN": "环境感知", "en-US": "Environment sensing" }, { "zh-CN": "除霜策略", "en-US": "Defog strategy" }, { "zh-CN": "驾驶监测", "en-US": "Driver monitoring" }],
    domains: ["visibilitySensing", "climateComfort", "driverMonitoring", "voiceHmi", "safetyEmergency"],
    capability: "visibility.activateDefog",
    command: "windshieldDefogActivate",
    target: "defogger-1",
    evidenceEvent: "WindshieldVisibilityRestored",
    deadlineTick: 24,
    occupants: 2,
    systems: 3
  },
  {
    id: "driver-fatigue-guardian",
    path: "scenarios/driver-fatigue-guardian.yaml",
    domain: { "zh-CN": "驾驶员监测", "en-US": "Driver monitoring" },
    title: { "zh-CN": "长途夜驾疲劳守护", "en-US": "Fatigue guardian on a night journey" },
    objective: { "zh-CN": "识别注意力下降并分级干预", "en-US": "Detect attention decay and escalate intervention" },
    risk: { "zh-CN": "微睡眠与接管失败", "en-US": "Microsleep and failed takeover" },
    trigger: { "zh-CN": "长途夜驾使驾驶员注意力每 3 tick 下降", "en-US": "Night driving reduces driver attention every three ticks" },
    coverage: [{ "zh-CN": "DMS", "en-US": "DMS" }, { "zh-CN": "分级提醒", "en-US": "Escalating alerts" }, { "zh-CN": "人机共驾", "en-US": "Shared control" }],
    domains: ["driverMonitoring", "healthWellness", "adasAutomation", "voiceHmi", "safetyEmergency"],
    capability: "driver.activateFatigueIntervention",
    command: "fatigueInterventionActivate",
    target: "dms-1",
    evidenceEvent: "DriverAttentionRestored",
    deadlineTick: 20,
    occupants: 2,
    systems: 2
  },
  {
    id: "child-left-behind",
    path: "scenarios/child-left-behind.yaml",
    domain: { "zh-CN": "儿童与生命体征", "en-US": "Child presence & vital safety" },
    title: { "zh-CN": "锁车后的儿童遗留预警", "en-US": "Child-left-behind protection after locking" },
    objective: { "zh-CN": "确认生命存在、降温并触达监护人", "en-US": "Confirm presence, cool the cabin, and contact the guardian" },
    risk: { "zh-CN": "密闭高温与响应延迟", "en-US": "Heat exposure and delayed response" },
    trigger: { "zh-CN": "锁车后舱温与儿童压力持续上升，普通通知可能被忽略", "en-US": "Cabin heat and child stress rise after locking while ordinary notifications may be missed" },
    coverage: [{ "zh-CN": "乘员检测", "en-US": "Presence detection" }, { "zh-CN": "远程通知", "en-US": "Remote notification" }, { "zh-CN": "救援升级", "en-US": "Rescue escalation" }],
    domains: ["occupantChild", "healthWellness", "climateComfort", "connectivityRemote", "safetyEmergency"],
    capability: "occupant.activateChildProtection",
    command: "childProtectionActivate",
    target: "occupant-radar-1",
    evidenceEvent: "ChildProtectionActivated",
    deadlineTick: 22,
    occupants: 2,
    systems: 4
  },
  {
    id: "medical-emergency",
    path: "scenarios/medical-emergency.yaml",
    domain: { "zh-CN": "健康与医疗救援", "en-US": "Health & medical response" },
    title: { "zh-CN": "乘员突发健康异常", "en-US": "Sudden occupant health emergency" },
    objective: { "zh-CN": "降低驾驶负荷、建立急救通话并导航救援", "en-US": "Reduce driver load, establish emergency contact, and route to care" },
    risk: { "zh-CN": "误判病情与信息遗漏", "en-US": "Misclassification and missing context" },
    trigger: { "zh-CN": "患者压力持续升高，同时驾驶员注意力因救援协调而下降", "en-US": "Patient stress rises while rescue coordination degrades driver attention" },
    coverage: [{ "zh-CN": "健康监测", "en-US": "Health monitoring" }, { "zh-CN": "紧急呼叫", "en-US": "Emergency calling" }, { "zh-CN": "导航联动", "en-US": "Navigation coordination" }],
    domains: ["healthWellness", "safetyEmergency", "navigationMobility", "connectivityRemote", "voiceHmi", "occupantChild"],
    capability: "health.activateMedicalResponse",
    command: "medicalResponseActivate",
    target: "emergency-call-1",
    evidenceEvent: "MedicalResponseActivated",
    deadlineTick: 22,
    occupants: 3,
    systems: 4
  },
  {
    id: "voice-privacy-conflict",
    path: "scenarios/voice-privacy-conflict.yaml",
    domain: { "zh-CN": "多用户交互与隐私", "en-US": "Multi-user interaction & privacy" },
    title: { "zh-CN": "家庭出行中的语音与隐私冲突", "en-US": "Voice and privacy conflict on a family trip" },
    objective: { "zh-CN": "正确识别说话人并保护个人消息", "en-US": "Identify speakers correctly and protect private messages" },
    risk: { "zh-CN": "越权披露与指令冲突", "en-US": "Unauthorized disclosure and command conflicts" },
    trigger: { "zh-CN": "四名乘员同时提出导航、消息和媒体请求，驾驶员持续分心", "en-US": "Four occupants issue concurrent navigation, messaging, and media requests that distract the driver" },
    coverage: [{ "zh-CN": "声纹识别", "en-US": "Voice identity" }, { "zh-CN": "隐私策略", "en-US": "Privacy policy" }, { "zh-CN": "多意图仲裁", "en-US": "Intent arbitration" }],
    domains: ["voiceHmi", "infotainmentMedia", "personalizationMultiUser", "cybersecurityPrivacy", "driverMonitoring", "navigationMobility"],
    capability: "privacy.activateMode",
    command: "privacyModeActivate",
    target: "voice-array-1",
    evidenceEvent: "PrivacyConflictContained",
    deadlineTick: 20,
    occupants: 4,
    systems: 4
  },
  {
    id: "ev-range-anxiety",
    path: "scenarios/ev-range-anxiety.yaml",
    domain: { "zh-CN": "能源与出行规划", "en-US": "Energy & journey planning" },
    title: { "zh-CN": "低电量山区改道", "en-US": "Low-battery mountain reroute" },
    objective: { "zh-CN": "解释续航变化并协商充电方案", "en-US": "Explain range changes and negotiate a charging plan" },
    risk: { "zh-CN": "续航不足与决策焦虑", "en-US": "Insufficient range and decision anxiety" },
    trigger: { "zh-CN": "低温、高海拔和强风同时造成舱温下降与续航焦虑", "en-US": "Cold, altitude, and strong wind combine to reduce cabin temperature and increase range anxiety" },
    coverage: [{ "zh-CN": "能量预测", "en-US": "Energy prediction" }, { "zh-CN": "路线规划", "en-US": "Route planning" }, { "zh-CN": "可解释交互", "en-US": "Explainable interaction" }],
    domains: ["energyCharging", "navigationMobility", "climateComfort", "connectivityRemote", "voiceHmi"],
    capability: "energy.acceptChargingPlan",
    command: "chargingPlanAccept",
    target: "navigation-1",
    evidenceEvent: "ChargingPlanAccepted",
    deadlineTick: 22,
    occupants: 2,
    systems: 4
  },
  {
    id: "adas-takeover-construction",
    path: "scenarios/adas-takeover-construction.yaml",
    domain: { "zh-CN": "辅助驾驶与接管", "en-US": "ADAS & takeover" },
    title: { "zh-CN": "施工区感知降级接管", "en-US": "Takeover under construction-zone degradation" },
    objective: { "zh-CN": "清晰传达系统边界并确认驾驶员接管", "en-US": "Communicate system limits and confirm driver takeover" },
    risk: { "zh-CN": "模式混淆与迟滞接管", "en-US": "Mode confusion and late takeover" },
    trigger: { "zh-CN": "施工区降水和感知压力要求驾驶员及时恢复人工控制", "en-US": "Construction-zone precipitation and perception demand require a timely return to manual control" },
    coverage: [{ "zh-CN": "传感器融合", "en-US": "Sensor fusion" }, { "zh-CN": "模式管理", "en-US": "Mode management" }, { "zh-CN": "接管闭环", "en-US": "Takeover loop" }],
    domains: ["adasAutomation", "visibilitySensing", "driverMonitoring", "safetyEmergency", "voiceHmi"],
    capability: "adas.acknowledgeTakeover",
    command: "adasTakeoverAcknowledge",
    target: "adas-controller-1",
    evidenceEvent: "AdasTakeoverCompleted",
    deadlineTick: 18,
    occupants: 2,
    systems: 5
  },
  {
    id: "cybersecurity-anomalous-control",
    path: "scenarios/cybersecurity-anomalous-control.yaml",
    domain: { "zh-CN": "网络安全与权限", "en-US": "Cybersecurity & authorization" },
    title: { "zh-CN": "异常远程控制请求", "en-US": "Anomalous remote-control request" },
    objective: { "zh-CN": "阻断越权动作、保留证据并维持安全功能", "en-US": "Block unauthorized actions, preserve evidence, and retain safe functions" },
    risk: { "zh-CN": "控制权劫持与服务降级", "en-US": "Control hijack and service degradation" },
    trigger: { "zh-CN": "异常远程控制请求触发鉴权、证据保留和网络隔离响应", "en-US": "An anomalous remote-control request triggers authentication, evidence retention, and network isolation" },
    coverage: [{ "zh-CN": "零信任鉴权", "en-US": "Zero-trust authorization" }, { "zh-CN": "安全降级", "en-US": "Safe degradation" }, { "zh-CN": "审计追踪", "en-US": "Audit trail" }],
    domains: ["cybersecurityPrivacy", "connectivityRemote", "safetyEmergency", "personalizationMultiUser"],
    capability: "cybersecurity.enterSafeMode",
    command: "cyberSafeModeActivate",
    target: "security-monitor-1",
    evidenceEvent: "CyberIncidentContained",
    deadlineTick: 16,
    occupants: 2,
    systems: 6
  }
];
