import { afterEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { SimulationSourcePanel } from "./SimulationSourcePanel";
import { I18nProvider } from "../i18n";
import { runnerClient } from "../runnerClient";
import { initialSimulationModel } from "../state/simulationReducer";

let container: HTMLDivElement | null = null;
let root: Root | null = null;

function render(dispatch: ReturnType<typeof vi.fn>) {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  act(() => {
    root!.render(
      <I18nProvider>
        <SimulationSourcePanel
          model={{ ...initialSimulationModel, state: "connectedIdle", serviceConnected: true, lastCursor: 0 }}
          dispatch={dispatch}
        />
      </I18nProvider>
    );
  });
  return container;
}

function emptyBatch(cursor = 0) {
  return { events: [], nextCursor: cursor, firstAvailableCursor: 0, resetRequired: false };
}

afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
  container = null;
  root = null;
  vi.restoreAllMocks();
  window.localStorage?.clear();
});

describe("SimulationSourcePanel auto-run", () => {
  it("adopts the Runner failure event when a live turn times out", async () => {
    const dispatch = vi.fn();
    vi.spyOn(runnerClient, "validateScenario").mockResolvedValue({
      id: "smoke-emergency-response",
      path: "scenarios/smoke-in-cockpit.yaml",
      schemaVersion: 1,
      scenarioHash: "hash",
      seed: 42,
      agentId: "cockpit-agent",
    });
    vi.spyOn(runnerClient, "createLiveRun").mockResolvedValue({ runId: "run-timeout", backend: "iota-core-acp" });
    vi.spyOn(runnerClient, "start").mockResolvedValue();
    vi.spyOn(runnerClient, "stepLive").mockRejectedValue(new Error("backend turn exceeded 60000ms"));
    vi.spyOn(runnerClient, "snapshot")
      .mockResolvedValueOnce(emptyBatch())
      .mockResolvedValueOnce(emptyBatch())
      .mockResolvedValueOnce(emptyBatch(3))
      .mockResolvedValueOnce({
        events: [{
          type: "SimulationError" as const,
          cursor: 4,
          error: {
            code: "LIVE_BACKEND_TURN_FAILED",
            message: "backend turn exceeded 60000ms",
            runId: "run-timeout",
            tick: 1,
            correlationId: "live-backend",
          },
        }],
        nextCursor: 5,
        firstAvailableCursor: 0,
        resetRequired: false,
      });
    const element = render(dispatch);

    await act(async () => {
      (element.querySelector('button[aria-label="一键运行"]') as HTMLButtonElement).click();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(dispatch).toHaveBeenCalledWith(expect.objectContaining({
      type: "runnerEvents",
      events: expect.arrayContaining([expect.objectContaining({ type: "SimulationError" })]),
    }));
    expect(dispatch).not.toHaveBeenCalledWith(expect.objectContaining({ type: "commandRejected" }));
  });

  it("ignores the step shortcut while auto-run is in flight to avoid a concurrent stepLive call", async () => {
    const dispatch = vi.fn();
    vi.spyOn(runnerClient, "validateScenario").mockResolvedValue({
      id: "smoke-emergency-response",
      path: "scenarios/smoke-in-cockpit.yaml",
      schemaVersion: 1,
      scenarioHash: "hash",
      seed: 42,
      agentId: "cockpit-agent",
    });
    vi.spyOn(runnerClient, "createLiveRun").mockResolvedValue({ runId: "run-concurrent", backend: "iota-core-acp" });
    vi.spyOn(runnerClient, "start").mockResolvedValue();
    let releaseStep: (() => void) | undefined;
    const stepLiveSpy = vi.spyOn(runnerClient, "stepLive").mockImplementation(
      () => new Promise((resolve) => {
        releaseStep = () => resolve({ status: "running" });
      })
    );
    vi.spyOn(runnerClient, "snapshot").mockResolvedValue(emptyBatch());

    const element = render(dispatch);

    await act(async () => {
      (element.querySelector('button[aria-label="一键运行"]') as HTMLButtonElement).click();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    // Auto-run's loop has issued exactly one in-flight stepLive call.
    expect(stepLiveSpy).toHaveBeenCalledTimes(1);

    // Pressing the step shortcut while auto-run is still awaiting that call
    // must not enqueue a second, concurrent stepLive request.
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "s" }));
      await Promise.resolve();
    });
    expect(stepLiveSpy).toHaveBeenCalledTimes(1);

    releaseStep?.();
    await act(async () => {
      await Promise.resolve();
    });
  });
});
