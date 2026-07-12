import { APP_CONFIG } from "../config/constants";
import { persistApprovalMode, persistRunId, persistScenario } from "../utils/storage";
import type {
  RunnerEvent,
  ScenarioSummary,
  SimulationError,
  SimulationModel
} from "../types/simulation";

export type SimulationAction =
  | { type: "connectRequested" }
  | { type: "connected" }
  | { type: "disconnected"; error?: SimulationError }
  | { type: "scenarioLoading" }
  | { type: "scenarioInvalid"; error: SimulationError }
  | { type: "runCreating" }
  | { type: "scenarioReady"; scenario: ScenarioSummary; runId?: string }
  | { type: "approvalModeChanged"; required: boolean }
  | { type: "replayDiffUpdated"; report: import("../types/simulation").RecordingDiff }
  | { type: "snapshotReset"; snapshot: import("../types/simulation").WorldSnapshot; cursor: number }
  | { type: "commandRejected"; error: SimulationError }
  | { type: "runnerEvent"; event: RunnerEvent };

export const initialSimulationModel: SimulationModel = {
  state: "disconnected",
  tick: 0,
  simTimeMs: 0,
  speed: 1,
  observations: [],
  events: [],
  toolCalls: [],
  actionResults: [],
  serviceConnected: false,
  approvalRequired: false
};

export function simulationReducer(
  state: SimulationModel,
  action: SimulationAction
): SimulationModel {
  switch (action.type) {
    case "connectRequested":
      return { ...state, state: "connecting", serviceConnected: false, error: undefined };
    case "connected":
      return { ...state, state: "connectedIdle", serviceConnected: true, error: undefined };
    case "disconnected":
      return {
        ...state,
        state: "disconnected",
        serviceConnected: false,
        error: action.error
      };
    case "scenarioLoading":
      return { ...state, state: "scenarioLoading", error: undefined };
    case "scenarioInvalid":
      return { ...state, state: "scenarioInvalid", error: action.error };
    case "runCreating":
      return { ...state, state: "runCreating", error: undefined };
    case "scenarioReady":
      if (action.scenario) persistScenario(action.scenario);
      if (action.runId) persistRunId(action.runId);
      return {
        ...state,
        state: "ready",
        scenario: action.scenario,
        runId: action.runId ?? state.runId,
        events: [],
        observations: [],
        actionResults: [],
        toolCalls: [],
        evaluation: undefined,
        tick: 0,
        simTimeMs: 0,
        error: undefined
      };
    case "approvalModeChanged":
      persistApprovalMode(action.required);
      return { ...state, approvalRequired: action.required };
    case "replayDiffUpdated":
      return { ...state, replayDiff: action.report };
    case "snapshotReset":
      return {
        ...state,
        runId: action.snapshot.runId,
        tick: action.snapshot.tick,
        simTimeMs: action.snapshot.simTimeMs,
        snapshot: action.snapshot,
        events: [],
        toolCalls: [],
        actionResults: [],
        lastCursor: action.cursor
      };
    case "commandRejected":
      return {
        ...state,
        state:
          state.state === "scenarioLoading" || state.state === "runCreating"
            ? "failed"
            : state.state,
        error: action.error
      };
    case "runnerEvent":
      return reduceRunnerEvent(state, action.event);
  }
}

function reduceRunnerEvent(state: SimulationModel, event: RunnerEvent): SimulationModel {
  switch (event.type) {
    case "SimulationStateChanged":
      return { ...state, state: event.state, runId: event.runId ?? state.runId };
    case "SimulationTickCommitted":
      return {
        ...state,
        state: state.state === "paused" || state.state === "replaying" ? state.state : "running",
        runId: event.snapshot.runId,
        tick: event.snapshot.tick,
        simTimeMs: event.snapshot.simTimeMs,
        snapshot: event.snapshot,
        lastCursor: event.cursor
      };
    case "SimulationEvent":
      return {
        ...state,
        events: [event.event, ...state.events].slice(0, APP_CONFIG.MAX_EVENTS),
        lastCursor: event.cursor
      };
    case "SimulationToolCall":
      return {
        ...state,
        toolCalls: [event.trace, ...state.toolCalls].slice(0, APP_CONFIG.MAX_TOOL_CALLS),
        lastCursor: event.cursor
      };
    case "SimulationActionResult":
      return {
        ...state,
        actionResults: [event.result, ...state.actionResults].slice(0, APP_CONFIG.MAX_ACTION_RESULTS),
        lastCursor: event.cursor
      };
    case "SimulationEvaluationUpdated":
      return { ...state, evaluation: event.evaluation, lastCursor: event.cursor };
    case "SimulationError":
      return {
        ...state,
        state: "failed",
        error: event.error,
        lastCursor: event.cursor ?? state.lastCursor
      };
  }
}

export function canStart(state: SimulationModel): boolean {
  return state.serviceConnected && ["ready", "paused", "stopped"].includes(state.state);
}

export function canPause(state: SimulationModel): boolean {
  return state.serviceConnected && ["running", "degraded"].includes(state.state);
}

export function canStep(state: SimulationModel): boolean {
  return state.serviceConnected && ["ready", "paused", "running"].includes(state.state);
}

export function canStop(state: SimulationModel): boolean {
  return state.serviceConnected && ["running", "paused", "degraded"].includes(state.state);
}
