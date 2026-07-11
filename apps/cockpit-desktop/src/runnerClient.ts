import { invoke } from "@tauri-apps/api/core";
import type { RunnerEvent, ScenarioSummary } from "./types/simulation";

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
  snapshot(cursor?: number): Promise<RunnerEvent[]>;
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
  async snapshot(cursor?: number) {
    return (await invokeRunner<RunnerEvent[]>("get_simulation_events", { cursor })) ?? [];
  }
};
