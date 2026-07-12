import { describe, it, expect } from "vitest";
import {
  simulationReducer,
  initialSimulationModel,
  canStart,
  canPause,
  canStep,
  canStop,
} from "./simulationReducer";
import type { SimulationAction } from "./simulationReducer";

describe("simulationReducer", () => {
  it("should handle connectRequested", () => {
    const action: SimulationAction = { type: "connectRequested" };
    const state = simulationReducer(initialSimulationModel, action);
    expect(state.state).toBe("connecting");
    expect(state.serviceConnected).toBe(false);
    expect(state.error).toBeUndefined();
  });

  it("should handle connected", () => {
    const action: SimulationAction = { type: "connected" };
    const state = simulationReducer(
      { ...initialSimulationModel, state: "connecting" },
      action
    );
    expect(state.state).toBe("connectedIdle");
    expect(state.serviceConnected).toBe(true);
  });

  it("should handle disconnected with error", () => {
    const error = {
      code: "TEST_ERROR",
      message: "Test error message",
      correlationId: "test-123",
    };
    const action: SimulationAction = { type: "disconnected", error };
    const state = simulationReducer(initialSimulationModel, action);
    expect(state.state).toBe("disconnected");
    expect(state.serviceConnected).toBe(false);
    expect(state.error).toEqual(error);
  });

  it("should handle scenarioReady", () => {
    const scenario = {
      id: "test-scenario",
      path: "/test/path",
      schemaVersion: 1,
      scenarioHash: "abc123",
      seed: 42,
      agentId: "test-agent",
    };
    const action: SimulationAction = {
      type: "scenarioReady",
      scenario,
      runId: "run-123",
    };
    const state = simulationReducer(initialSimulationModel, action);
    expect(state.state).toBe("ready");
    expect(state.scenario).toEqual(scenario);
    expect(state.runId).toBe("run-123");
    expect(state.tick).toBe(0);
    expect(state.events).toEqual([]);
  });

  it("should handle approvalModeChanged", () => {
    const action: SimulationAction = {
      type: "approvalModeChanged",
      required: true,
    };
    const state = simulationReducer(initialSimulationModel, action);
    expect(state.approvalRequired).toBe(true);
  });

  it("should handle SimulationStateChanged event", () => {
    const event = {
      type: "SimulationStateChanged" as const,
      state: "running" as const,
      runId: "run-123",
    };
    const action: SimulationAction = { type: "runnerEvent", event };
    const state = simulationReducer(initialSimulationModel, action);
    expect(state.state).toBe("running");
    expect(state.runId).toBe("run-123");
  });

  it("should handle SimulationTickCommitted event", () => {
    const snapshot = {
      runId: "run-123",
      tick: 5,
      simTimeMs: 500,
      version: 1,
      environment: {
        temperatureC: 22,
        humidityPct: 50,
        visibility: 0.9,
        smokeDensity: 0.1,
        lightingLux: 300,
        noiseDb: 40,
        fireActive: false,
      },
      pilot: {
        stress: 0.2,
        fatigue: 0.1,
        health: 1.0,
        attention: 0.8,
        location: "cabin",
      },
      engine: {
        health: 0.95,
        powerState: "running",
        lifecycle: "operational",
        faults: [],
        capabilities: ["thrust"],
        shutdown: false,
      },
      alarm: {
        active: false,
        volumeDb: 0,
      },
    };
    const event = {
      type: "SimulationTickCommitted" as const,
      snapshot,
      cursor: 10,
    };
    const action: SimulationAction = { type: "runnerEvent", event };
    const state = simulationReducer(initialSimulationModel, action);
    expect(state.tick).toBe(5);
    expect(state.simTimeMs).toBe(500);
    expect(state.snapshot).toEqual(snapshot);
    expect(state.lastCursor).toBe(10);
  });

  it("should transition to failed on a SimulationError event", () => {
    const event = {
      type: "SimulationError" as const,
      cursor: 7,
      error: {
        code: "RECORDING_QUEUE_OVERFLOW",
        message: "recording queue reached its bounded capacity",
        correlationId: "recording-queue",
      },
    };
    const state = simulationReducer(initialSimulationModel, {
      type: "runnerEvent",
      event,
    });
    expect(state.state).toBe("failed");
    expect(state.error?.code).toBe("RECORDING_QUEUE_OVERFLOW");
    expect(state.lastCursor).toBe(7);
  });

  it("should preserve paused/replaying state across tick commits", () => {
    const snapshot = {
      runId: "run-1",
      tick: 3,
      simTimeMs: 300,
      version: 1,
      environment: {
        temperatureC: 22,
        humidityPct: 50,
        visibility: 0.9,
        smokeDensity: 0.1,
        lightingLux: 300,
        noiseDb: 40,
        fireActive: false,
      },
      pilot: { stress: 0.2, fatigue: 0.1, health: 1, attention: 0.8, location: "cabin" },
      engine: {
        health: 0.95,
        powerState: "running",
        lifecycle: "operational",
        faults: [],
        capabilities: ["thrust"],
        shutdown: false,
      },
      alarm: { active: false, volumeDb: 0 },
    };
    const paused = simulationReducer(
      { ...initialSimulationModel, state: "paused" },
      { type: "runnerEvent", event: { type: "SimulationTickCommitted", snapshot, cursor: 1 } }
    );
    expect(paused.state).toBe("paused");
  });

  it("should cap the event log at MAX_EVENTS", async () => {
    const { APP_CONFIG } = await import("../config/constants");
    let state = initialSimulationModel;
    for (let i = 0; i < APP_CONFIG.MAX_EVENTS + 10; i += 1) {
      state = simulationReducer(state, {
        type: "runnerEvent",
        event: {
          type: "SimulationEvent",
          cursor: i,
          event: {
            eventId: `evt-${i}`,
            eventType: "TestEvent",
            runId: "run-1",
            tick: i,
            source: "test",
            priority: 1,
            sequence: i,
            correlationId: `corr-${i}`,
            payload: { message: "m" },
          },
        },
      });
    }
    expect(state.events.length).toBe(APP_CONFIG.MAX_EVENTS);
    // Newest first.
    expect(state.events[0].eventId).toBe(`evt-${APP_CONFIG.MAX_EVENTS + 9}`);
  });

  it("should reset event/cursor state on snapshotReset", () => {
    const snapshot = {
      runId: "run-9",
      tick: 12,
      simTimeMs: 1200,
      version: 3,
      environment: {
        temperatureC: 22,
        humidityPct: 50,
        visibility: 0.9,
        smokeDensity: 0.1,
        lightingLux: 300,
        noiseDb: 40,
        fireActive: false,
      },
      pilot: { stress: 0.2, fatigue: 0.1, health: 1, attention: 0.8, location: "cabin" },
      engine: {
        health: 0.95,
        powerState: "running",
        lifecycle: "operational",
        faults: [],
        capabilities: ["thrust"],
        shutdown: false,
      },
      alarm: { active: false, volumeDb: 0 },
    };
    const seeded = {
      ...initialSimulationModel,
      events: [
        {
          eventId: "old",
          eventType: "Old",
          runId: "run-9",
          tick: 1,
          source: "s",
          priority: 1,
          sequence: 1,
          correlationId: "c",
          payload: { message: "m" },
        },
      ],
    };
    const state = simulationReducer(seeded, {
      type: "snapshotReset",
      snapshot,
      cursor: 42,
    });
    expect(state.events).toEqual([]);
    expect(state.tick).toBe(12);
    expect(state.lastCursor).toBe(42);
    expect(state.runId).toBe("run-9");
  });
});

describe("state guards", () => {
  it("canStart should return true for ready, paused, and stopped states", () => {
    expect(
      canStart({ ...initialSimulationModel, state: "ready", serviceConnected: true })
    ).toBe(true);
    expect(
      canStart({ ...initialSimulationModel, state: "paused", serviceConnected: true })
    ).toBe(true);
    expect(
      canStart({ ...initialSimulationModel, state: "stopped", serviceConnected: true })
    ).toBe(true);
    expect(
      canStart({ ...initialSimulationModel, state: "running", serviceConnected: true })
    ).toBe(false);
  });

  it("canPause should return true for running and degraded states", () => {
    expect(
      canPause({ ...initialSimulationModel, state: "running", serviceConnected: true })
    ).toBe(true);
    expect(
      canPause({ ...initialSimulationModel, state: "degraded", serviceConnected: true })
    ).toBe(true);
    expect(
      canPause({ ...initialSimulationModel, state: "paused", serviceConnected: true })
    ).toBe(false);
  });

  it("canStep should return true for ready, paused, and running states", () => {
    expect(
      canStep({ ...initialSimulationModel, state: "ready", serviceConnected: true })
    ).toBe(true);
    expect(
      canStep({ ...initialSimulationModel, state: "paused", serviceConnected: true })
    ).toBe(true);
    expect(
      canStep({ ...initialSimulationModel, state: "running", serviceConnected: true })
    ).toBe(true);
    expect(
      canStep({ ...initialSimulationModel, state: "stopped", serviceConnected: true })
    ).toBe(false);
  });

  it("canStop should return true for running, paused, and degraded states", () => {
    expect(
      canStop({ ...initialSimulationModel, state: "running", serviceConnected: true })
    ).toBe(true);
    expect(
      canStop({ ...initialSimulationModel, state: "paused", serviceConnected: true })
    ).toBe(true);
    expect(
      canStop({ ...initialSimulationModel, state: "degraded", serviceConnected: true })
    ).toBe(true);
    expect(
      canStop({ ...initialSimulationModel, state: "ready", serviceConnected: true })
    ).toBe(false);
  });

  it("should require serviceConnected for all guards", () => {
    expect(
      canStart({ ...initialSimulationModel, state: "ready", serviceConnected: false })
    ).toBe(false);
    expect(
      canPause({ ...initialSimulationModel, state: "running", serviceConnected: false })
    ).toBe(false);
    expect(
      canStep({ ...initialSimulationModel, state: "ready", serviceConnected: false })
    ).toBe(false);
    expect(
      canStop({ ...initialSimulationModel, state: "running", serviceConnected: false })
    ).toBe(false);
  });
});
