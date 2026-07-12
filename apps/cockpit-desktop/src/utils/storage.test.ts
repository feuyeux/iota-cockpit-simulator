import { describe, it, expect, beforeEach, vi } from "vitest";
import {
  loadPersistedSession,
  persistScenario,
  persistRunId,
  persistApprovalMode,
  clearPersistedSession,
} from "./storage";
import type { ScenarioSummary } from "../types/simulation";

// Mock localStorage
const mockLocalStorage: Record<string, string> = {};

beforeEach(() => {
  Object.keys(mockLocalStorage).forEach((key) => delete mockLocalStorage[key]);
  
  globalThis.localStorage = {
    getItem: vi.fn((key: string) => mockLocalStorage[key] ?? null),
    setItem: vi.fn((key: string, value: string) => {
      mockLocalStorage[key] = value;
    }),
    removeItem: vi.fn((key: string) => {
      delete mockLocalStorage[key];
    }),
    clear: vi.fn(() => {
      Object.keys(mockLocalStorage).forEach((key) => delete mockLocalStorage[key]);
    }),
    key: vi.fn(),
    length: 0,
  } as Storage;
});

describe("storage utilities", () => {
  it("should return null when no session is persisted", () => {
    const session = loadPersistedSession();
    expect(session).toBeNull();
  });

  it("should persist and load scenario", () => {
    const scenario: ScenarioSummary = {
      id: "test-scenario",
      path: "/test/path",
      schemaVersion: 1,
      scenarioHash: "hash123",
      seed: 42,
      agentId: "agent-1",
    };

    persistScenario(scenario);
    const session = loadPersistedSession();

    expect(session).not.toBeNull();
    expect(session?.scenario).toEqual(scenario);
  });

  it("should persist and load runId", () => {
    const scenario: ScenarioSummary = {
      id: "test-scenario",
      path: "/test/path",
      schemaVersion: 1,
      scenarioHash: "hash123",
      seed: 42,
      agentId: "agent-1",
    };

    persistScenario(scenario);
    persistRunId("run-123");
    
    const session = loadPersistedSession();
    expect(session?.runId).toBe("run-123");
  });

  it("should persist and load approval mode", () => {
    const scenario: ScenarioSummary = {
      id: "test-scenario",
      path: "/test/path",
      schemaVersion: 1,
      scenarioHash: "hash123",
      seed: 42,
      agentId: "agent-1",
    };

    persistScenario(scenario);
    persistApprovalMode(true);
    
    const session = loadPersistedSession();
    expect(session?.approvalRequired).toBe(true);
  });

  it("should clear persisted session", () => {
    const scenario: ScenarioSummary = {
      id: "test-scenario",
      path: "/test/path",
      schemaVersion: 1,
      scenarioHash: "hash123",
      seed: 42,
      agentId: "agent-1",
    };

    persistScenario(scenario);
    persistRunId("run-123");
    clearPersistedSession();
    
    const session = loadPersistedSession();
    expect(session).toBeNull();
  });

  it("should handle corrupted storage data gracefully", () => {
    mockLocalStorage["cockpit:lastScenario"] = "invalid-json{";
    
    const session = loadPersistedSession();
    expect(session).toBeNull();
  });
});
