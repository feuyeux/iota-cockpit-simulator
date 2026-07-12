import { APP_CONFIG } from "../config/constants";
import type { ScenarioSummary } from "../types/simulation";

export interface PersistedSession {
  scenario?: ScenarioSummary;
  runId?: string;
  approvalRequired: boolean;
}

export function loadPersistedSession(): PersistedSession | null {
  try {
    const scenarioJson = localStorage.getItem(APP_CONFIG.STORAGE_KEY_LAST_SCENARIO);
    const runId = localStorage.getItem(APP_CONFIG.STORAGE_KEY_LAST_RUN);
    const approvalRequired = localStorage.getItem(APP_CONFIG.STORAGE_KEY_APPROVAL_MODE);

    if (!scenarioJson) return null;

    return {
      scenario: JSON.parse(scenarioJson) as ScenarioSummary,
      runId: runId ?? undefined,
      approvalRequired: approvalRequired === "true",
    };
  } catch (error) {
    console.warn("Failed to load persisted session:", error);
    return null;
  }
}

export function persistScenario(scenario: ScenarioSummary): void {
  try {
    localStorage.setItem(APP_CONFIG.STORAGE_KEY_LAST_SCENARIO, JSON.stringify(scenario));
  } catch (error) {
    console.warn("Failed to persist scenario:", error);
  }
}

export function persistRunId(runId: string): void {
  try {
    localStorage.setItem(APP_CONFIG.STORAGE_KEY_LAST_RUN, runId);
  } catch (error) {
    console.warn("Failed to persist run ID:", error);
  }
}

export function persistApprovalMode(required: boolean): void {
  try {
    localStorage.setItem(APP_CONFIG.STORAGE_KEY_APPROVAL_MODE, String(required));
  } catch (error) {
    console.warn("Failed to persist approval mode:", error);
  }
}

export function clearPersistedSession(): void {
  try {
    localStorage.removeItem(APP_CONFIG.STORAGE_KEY_LAST_SCENARIO);
    localStorage.removeItem(APP_CONFIG.STORAGE_KEY_LAST_RUN);
  } catch (error) {
    console.warn("Failed to clear persisted session:", error);
  }
}
