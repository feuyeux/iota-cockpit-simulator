export type RunState =
  | "disconnected"
  | "connecting"
  | "connectedIdle"
  | "scenarioLoading"
  | "scenarioInvalid"
  | "ready"
  | "runCreating"
  | "running"
  | "paused"
  | "degraded"
  | "replaying"
  | "completed"
  | "stopped"
  | "failed";

export interface SimulationError {
  code: string;
  message: string;
  details?: unknown;
  runId?: string;
  tick?: number;
  correlationId: string;
}

export interface ScenarioSummary {
  id: string;
  path: string;
  schemaVersion: number;
  scenarioHash: string;
  seed: number;
  agentId: string;
}

export interface LiveRunSummary {
  runId: string;
  backend: string;
}

export interface SensorQuality {
  visibilityQuality: number;
  audioQuality: number;
  confidence: number;
  degraded: boolean;
}

export interface Observation {
  observationId: string;
  runId: string;
  agentId: string;
  sensorId: string;
  observedTick: number;
  deliveredTick: number;
  visibleEntities: string[];
  alerts: string[];
  actionResults: string[];
  confidence: number;
  quality: SensorQuality;
}

export interface SimulationEvent {
  eventId: string;
  eventType: string;
  runId: string;
  tick: number;
  source: string;
  priority: number;
  sequence: number;
  correlationId: string;
  payload: {
    message: string;
    target?: string;
    value?: number | null;
  };
}

export interface ActionResult {
  request: {
    requestId: string;
    agentId: string;
    target: string;
    command:
      | "engineShutdown"
      | "alarmActivate"
      | "climateComfortRestore"
      | "windshieldDefogActivate"
      | "fatigueInterventionActivate"
      | "childProtectionActivate"
      | "medicalResponseActivate"
      | "privacyModeActivate"
      | "chargingPlanAccept"
      | "adasTakeoverAcknowledge"
      | "cyberSafeModeActivate";
    expectedStateVersion: number;
    expiresAtTick: number;
    correlationId: string;
  };
  status: "pendingApproval" | "applied" | "rejected" | "superseded";
  errorCode?: string;
  runId: string;
  tick: number;
  correlationId: string;
}

export interface BigFiveTraits {
  openness: number;
  conscientiousness: number;
  extraversion: number;
  agreeableness: number;
  neuroticism: number;
}

export interface Persona {
  name: string;
  role: string;
  background: string;
  traits: BigFiveTraits;
  relationships: string[];
}

export interface NeedsState {
  comfort: number;
  safety: number;
  social: number;
}

export interface PerceivedEvent {
  originTick: number;
  availableAtTick: number;
  source: string;
  kind: string;
  summary: string;
}

export interface HumanState {
  id: string;
  persona: Persona;
  needs: NeedsState;
  stress: number;
  fatigue: number;
  health: number;
  attention: number;
  location: string;
  goal: string;
  shortTermMemory: PerceivedEvent[];
  longTermMemory: string[];
}

export interface DeviceState {
  id: string;
  health: number;
  powerState: string;
  lifecycle: string;
  faults: string[];
  capabilities: string[];
  shutdown: boolean;
}

export interface OuterEnvironmentState {
  externalTemperatureC: number;
  altitudeM: number;
  windSpeedKmh: number;
  precipitation: number;
  threatActive: boolean;
}

export interface WorldSnapshot {
  runId: string;
  tick: number;
  simTimeMs: number;
  version: number;
  outerEnvironment: OuterEnvironmentState;
  environment: {
    temperatureC: number;
    humidityPct: number;
    visibility: number;
    smokeDensity: number;
    lightingLux: number;
    noiseDb: number;
    fireActive: boolean;
  };
  humans: HumanState[];
  devices: DeviceState[];
  alarm: {
    active: boolean;
    volumeDb: number;
  };
  cockpitSystems?: {
    climate: {
      comfortTargetC: number | null;
      coolingActive: boolean;
      defogActive: boolean;
      seatVentilationActive: boolean;
    };
    driverAssistance: {
      fatigueInterventionActive: boolean;
      takeoverAcknowledged: boolean;
      takeoverHmiActive: boolean;
    };
    occupantCare: {
      childProtectionActive: boolean;
      medicalResponseActive: boolean;
      emergencyContacted: boolean;
      guardianNotified: boolean;
      remoteUnlockRequested: boolean;
    };
    experience: {
      privacyModeActive: boolean;
      chargingPlanAccepted: boolean;
      mediaSessionsIsolated: boolean;
      occupantProfilesIsolated: boolean;
    };
    mobility: {
      emergencyRouteActive: boolean;
      chargingRouteActive: boolean;
      chargerServiceConnected: boolean;
    };
    connectivity: {
      emergencyCallActive: boolean;
      remoteServicesIsolated: boolean;
      trustedLocalAlertActive: boolean;
    };
    cybersecurity: {
      safeModeActive: boolean;
      networkIsolated: boolean;
      identityVerified: boolean;
    };
  };
}

export interface EvaluationResult {
  passed: boolean;
  score: number;
  evidenceEventIds: string[];
  firstFailureTick: number | null;
  explanation: string;
  taskPassed?: boolean;
  taskScore?: number;
  safetyPassed?: boolean;
  trajectoryPassed?: boolean;
  safetyViolations?: SafetyViolation[];
  trajectory?: TrajectoryMetrics;
  executionPassed?: boolean;
  executionError?: string;
  ruleResults?: RuleEvaluationResult[];
}

export interface SafetyViolation {
  tick: number;
  requestId: string;
  code: string;
}

export interface TrajectoryMetrics {
  actionRequests: number;
  appliedActions: number;
  rejectedActions: number;
  sideEffectToolCalls: number;
  deniedToolCalls: number;
  alertTickExposure?: number;
  firstAppliedActionTick?: number | null;
}

export interface RuleEvaluationResult {
  ruleId: string;
  deadlineTick: number;
  result: EvaluationResult;
}

export type EvaluationVerdict = "pass" | "fail" | "inconclusive";

export interface EvidenceReference {
  tick: number;
  entityId?: string;
  eventId?: string;
  kind: string;
}

export interface JudgeProvenance {
  judgeId: string;
  model: string;
  promptHash: string;
  rubricHash: string;
  schemaHash: string;
}

export interface JudgeDecision {
  verdict: EvaluationVerdict;
  confidence: number;
  explanation: string;
  evidence: EvidenceReference[];
  provenance: JudgeProvenance;
}

export interface RuleVerdict {
  ruleId: string;
  deadlineTick: number;
  verdict: EvaluationVerdict;
  result: EvaluationResult;
}

export interface EvidenceVerdict {
  schemaVersion: number;
  verdict: EvaluationVerdict;
  rubricId: string;
  rubricVersion: string;
  rubricHash: string;
  inputHash: string;
  schemaHash: string;
  deterministicResults: RuleVerdict[];
  evidence: EvidenceReference[];
  judges: JudgeDecision[];
  judgeDisagreement: boolean;
  releaseGatePassed: boolean;
  explanation: string;
}

export interface EvaluationReportRecord {
  id: string;
  createdAtMs: number;
  runId: string;
  scenarioId: string;
  report: EvidenceVerdict;
}

export interface RecordingMetrics {
  ticks: number;
  events: number;
  toolCalls: number;
  actionResults: number;
  stateDiffs: number;
}

export interface ReplayTickDifference {
  tick: number;
  sourceSnapshotHash?: string;
  candidateSnapshotHash?: string;
  eventsMatch: boolean;
  toolCallsMatch: boolean;
  actionResultsMatch: boolean;
  stateDiffsMatch: boolean;
}

export interface RecordingDiff {
  equivalent: boolean;
  sourceFinalSnapshotHash?: string;
  candidateFinalSnapshotHash?: string;
  sourceMetrics: RecordingMetrics;
  candidateMetrics: RecordingMetrics;
  firstDivergence?: ReplayTickDifference;
  tickDifferences: ReplayTickDifference[];
  truncated: boolean;
}

export interface ToolCallTrace {
  callId: string;
  toolName: string;
  runId: string;
  agentId: string;
  tick: number;
  correlationId: string;
  arguments: unknown;
  result: unknown;
  sideEffect: boolean;
  allowed: boolean;
}

export interface HumanDecision {
  utterance?: string;
  actions: Array<{ target: string; command: string }>;
  internalStateDelta: {
    stress?: number;
    attention?: number;
  };
  narrative: string;
}

export interface HumanToolCall {
  tool: string;
  arguments: unknown;
}

export interface HumanTurnEvidence {
  humanId: string;
  decision: HumanDecision;
  toolCalls: HumanToolCall[];
  latencyMs?: number;
}

export interface HumanTurnTrace {
  tick: number;
  backend: string;
  evidence: HumanTurnEvidence;
}

export interface SimulatorEventBatch {
  events: SimulatorEvent[];
  nextCursor: number;
  firstAvailableCursor: number;
  resetRequired: boolean;
}

export interface SimulationModel {
  state: RunState;
  scenario?: ScenarioSummary;
  runId?: string;
  tick: number;
  simTimeMs: number;
  speed: number;
  snapshot?: WorldSnapshot;
  observations: Observation[];
  events: SimulationEvent[];
  toolCalls: ToolCallTrace[];
  humanTurns: HumanTurnTrace[];
  actionResults: ActionResult[];
  evaluation?: EvaluationResult;
  replayDiff?: RecordingDiff;
  error?: SimulationError;
  serviceConnected: boolean;
  approvalRequired: boolean;
  backend?: string;
  lastCursor?: number;
}

export type SimulatorEvent =
  | { type: "SimulationStateChanged"; state: RunState; runId?: string }
  | {
      type: "SimulationTickCommitted";
      runId: string;
      tick: number;
      simTimeMs: number;
      version: number;
      cursor: number;
    }
  | { type: "SimulationEvent"; event: SimulationEvent; cursor: number }
  | { type: "SimulationToolCall"; trace: ToolCallTrace; cursor: number }
  | {
      type: "SimulationHumanTurn";
      tick: number;
      backend: string;
      evidence: HumanTurnEvidence;
      cursor: number;
    }
  | { type: "SimulationActionResult"; result: ActionResult; cursor: number }
  | { type: "SimulationEvaluationUpdated"; evaluation: EvaluationResult; cursor: number }
  | { type: "SimulationError"; error: SimulationError; cursor?: number };
