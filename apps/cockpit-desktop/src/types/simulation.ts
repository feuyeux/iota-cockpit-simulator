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
    value?: number;
  };
}

export interface ActionResult {
  request: {
    requestId: string;
    agentId: string;
    target: string;
    command: "engineShutdown" | "alarmActivate";
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

export interface WorldSnapshot {
  runId: string;
  tick: number;
  simTimeMs: number;
  version: number;
  environment: {
    temperatureC: number;
    humidityPct: number;
    visibility: number;
    smokeDensity: number;
    lightingLux: number;
    noiseDb: number;
    fireActive: boolean;
  };
  pilot: {
    stress: number;
    fatigue: number;
    health: number;
    attention: number;
    location: string;
  };
  engine: {
    health: number;
    powerState: string;
    lifecycle: string;
    faults: string[];
    capabilities: string[];
    shutdown: boolean;
  };
  alarm: {
    active: boolean;
    volumeDb: number;
  };
}

export interface EvaluationResult {
  passed: boolean;
  score: number;
  evidenceEventIds: string[];
  firstFailureTick?: number;
  explanation: string;
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

export interface RunnerEventBatch {
  events: RunnerEvent[];
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
  actionResults: ActionResult[];
  evaluation?: EvaluationResult;
  replayDiff?: RecordingDiff;
  error?: SimulationError;
  serviceConnected: boolean;
  approvalRequired: boolean;
  lastCursor?: number;
}

export type RunnerEvent =
  | { type: "SimulationStateChanged"; state: RunState; runId?: string }
  | { type: "SimulationTickCommitted"; snapshot: WorldSnapshot; cursor: number }
  | { type: "SimulationEvent"; event: SimulationEvent; cursor: number }
  | { type: "SimulationToolCall"; trace: ToolCallTrace; cursor: number }
  | { type: "SimulationActionResult"; result: ActionResult; cursor: number }
  | { type: "SimulationEvaluationUpdated"; evaluation: EvaluationResult; cursor: number }
  | { type: "SimulationError"; error: SimulationError; cursor?: number };
