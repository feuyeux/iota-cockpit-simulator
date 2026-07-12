import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  RecordingDiff,
  RunnerEventBatch,
  ScenarioSummary,
  WorldSnapshot
} from "./types/simulation";

function isTauri(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

function invokeRunner<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) return Promise.resolve(undefined as T);
  return invoke<T>(command, args);
}

export interface RunnerClient {
  connect(): Promise<void>;
  validateScenario(path: string): Promise<ScenarioSummary>;
  createRun(path: string): Promise<string>;
  start(): Promise<void>;
  pause(): Promise<void>;
  step(): Promise<void>;
  stop(): Promise<void>;
  resume(scenarioPath: string, runId: string): Promise<void>;
  approveAction(requestId: string): Promise<unknown>;
  rejectAction(requestId: string, reason?: string): Promise<unknown>;
  cancelAgentTurn(): Promise<void>;
  setApprovalRequired(required: boolean): Promise<void>;
  startReplay(scenarioPath: string, recordingPath: string): Promise<unknown>;
  diffRecordings(sourceRecordingPath: string, candidateRecordingPath: string): Promise<RecordingDiff>;
  snapshot(cursor?: number): Promise<RunnerEventBatch>;
  simulationSnapshot(): Promise<WorldSnapshot>;
  openScenarioFilePicker(): Promise<string | null>;
  openRecordingFilePicker(): Promise<string | null>;
}

export const runnerClient: RunnerClient = {
  async connect() {
    await invokeRunner<void>("connect_runner");
  },
  async validateScenario(path: string) {
    if (!isTauri()) {
      return {
        id: "smoke-in-cockpit",
        path,
        schemaVersion: 1,
        scenarioHash: "dev-preview",
        seed: 42,
        agentId: "cockpit-agent"
      };
    }
    return invokeRunner("validate_scenario", { path });
  },
  async createRun(path: string) {
    if (!isTauri()) return "preview-run";
    return invokeRunner<string>("create_simulation_run", { path });
  },
  async start() {
    await invokeRunner<void>("start_simulation");
  },
  async pause() {
    await invokeRunner<void>("pause_simulation");
  },
  async step() {
    await invokeRunner<void>("step_simulation");
  },
  async stop() {
    await invokeRunner<void>("stop_simulation");
  },
  async resume(scenarioPath, runId) {
    await invokeRunner<void>("resume_simulation", { scenarioPath, runId });
  },
  async approveAction(requestId) {
    return invokeRunner("approve_action", { requestId });
  },
  async rejectAction(requestId, reason) {
    return invokeRunner("reject_action", { requestId, reason });
  },
  async cancelAgentTurn() {
    await invokeRunner<void>("cancel_agent_turn");
  },
  async setApprovalRequired(required) {
    await invokeRunner<void>("set_approval_required", { required });
  },
  async startReplay(scenarioPath, recordingPath) {
    return invokeRunner("start_replay", { scenarioPath, recordingPath });
  },
  async diffRecordings(sourceRecordingPath, candidateRecordingPath) {
    if (!isTauri()) {
      return {
        equivalent: sourceRecordingPath === candidateRecordingPath,
        sourceMetrics: { ticks: 0, events: 0, toolCalls: 0, actionResults: 0, stateDiffs: 0 },
        candidateMetrics: { ticks: 0, events: 0, toolCalls: 0, actionResults: 0, stateDiffs: 0 },
        tickDifferences: [],
        truncated: false
      };
    }
    return invokeRunner("diff_recordings", { sourceRecordingPath, candidateRecordingPath });
  },
  async snapshot(cursor?: number) {
    return (await invokeRunner<RunnerEventBatch>("get_simulation_events", { cursor })) ?? {
      events: [],
      nextCursor: cursor ?? 0,
      firstAvailableCursor: cursor ?? 0,
      resetRequired: false
    };
  },
  async simulationSnapshot() {
    return invokeRunner<WorldSnapshot>("get_simulation_snapshot");
  },
  async openScenarioFilePicker() {
    if (!isTauri()) return null;
    const result = await open({
      multiple: false,
      directory: false,
      filters: [
        { name: "YAML Scenarios", extensions: ["yaml", "yml"] },
        { name: "All Files", extensions: ["*"] },
      ],
    });
    if (!result) return null;
    if (typeof result === "string") return result;
    return (result as { path: string }).path ?? null;
  },
  async openRecordingFilePicker() {
    if (!isTauri()) return null;
    const result = await open({
      multiple: false,
      directory: false,
      filters: [
        { name: "Recording Files", extensions: ["json", "jsonl"] },
        { name: "All Files", extensions: ["*"] },
      ],
    });
    if (!result) return null;
    if (typeof result === "string") return result;
    return (result as { path: string }).path ?? null;
  },
};
