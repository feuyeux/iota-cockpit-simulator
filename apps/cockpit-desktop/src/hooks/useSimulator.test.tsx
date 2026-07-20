import { afterEach, describe, expect, it, vi } from "vitest";
import { act, useReducer } from "react";
import { createRoot, type Root } from "react-dom/client";
import { useSimulator } from "./useSimulator";
import { I18nProvider } from "../i18n";
import { simulatorClient } from "../simulatorClient";
import { initialSimulationModel, simulationReducer } from "../state/simulationReducer";

let container: HTMLDivElement | null = null;
let root: Root | null = null;

function Harness() {
  const [model, dispatch] = useReducer(simulationReducer, {
    ...initialSimulationModel,
    state: "running" as const,
    serviceConnected: true,
    runId: "run-timeout",
    lastCursor: 3,
  });
  const { runCommand } = useSimulator(model, dispatch);

  return (
    <>
      <button onClick={() => void runCommand(() => Promise.reject(new Error("transport rejected")))}>Run</button>
      <output data-testid="state">{model.state}</output>
      <output data-testid="error">{model.error?.code ?? ""}:{model.error?.message ?? ""}</output>
    </>
  );
}

function render() {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  act(() => {
    root!.render(<I18nProvider><Harness /></I18nProvider>);
  });
  return container;
}

afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
  container = null;
  root = null;
  vi.restoreAllMocks();
  window.localStorage.clear();
});

describe("useSimulator", () => {
  it("keeps the Simulator timeout error and failed terminal state after a command rejection", async () => {
    vi.spyOn(simulatorClient, "snapshot").mockResolvedValue({
      events: [
        {
          type: "SimulationError",
          cursor: 4,
          error: {
            code: "LIVE_BACKEND_TURN_FAILED",
            message: "backend turn exceeded 60000ms",
            runId: "run-timeout",
            tick: 7,
            correlationId: "live-backend",
          },
        },
        { type: "SimulationStateChanged", state: "failed", runId: "run-timeout" },
      ],
      nextCursor: 5,
      firstAvailableCursor: 0,
      resetRequired: false,
    });
    const element = render();

    await act(async () => {
      (element.querySelector("button") as HTMLButtonElement).click();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(element.querySelector('[data-testid="state"]')?.textContent).toBe("failed");
    expect(element.querySelector('[data-testid="error"]')?.textContent).toContain("LIVE_BACKEND_TURN_FAILED");
    expect(element.querySelector('[data-testid="error"]')?.textContent).toContain("backend turn exceeded 60000ms");
  });
});
