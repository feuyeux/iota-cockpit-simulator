import { describe, it, expect, afterEach } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { SimulationWorldView } from "./SimulationWorldView";
import { initialSimulationModel } from "../state/simulationReducer";
import type { Observation, SimulationModel } from "../types/simulation";

// Component tests render into jsdom via react-dom/client so no extra test
// dependency (react-testing-library) is required.

let container: HTMLDivElement | null = null;
let root: Root | null = null;

function render(model: SimulationModel) {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  act(() => {
    root!.render(<SimulationWorldView model={model} />);
  });
  return container;
}

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  container?.remove();
  container = null;
  root = null;
});

function observation(degraded: boolean): Observation {
  return {
    observationId: "obs-1",
    runId: "run-1",
    agentId: "agent-1",
    sensorId: "sensor-1",
    observedTick: 1,
    deliveredTick: 1,
    visibleEntities: [],
    alerts: [],
    actionResults: [],
    confidence: 0.5,
    quality: {
      visibilityQuality: 0.4,
      audioQuality: 0.6,
      confidence: 0.5,
      degraded,
    },
  } as Observation;
}

describe("SimulationWorldView", () => {
  it("hides Ground Truth and shows no degradation banner by default", () => {
    const el = render(initialSimulationModel);
    expect(el.textContent).toContain("Ground Truth hidden");
    expect(el.textContent).not.toContain("Sensor degraded");
  });

  it("shows the degradation banner and sensor quality when degraded", () => {
    const el = render({
      ...initialSimulationModel,
      observations: [observation(true)],
    });
    expect(el.textContent).toContain("Sensor degraded");
    expect(el.textContent).toContain("Visibility: 40%");
    expect(el.textContent).toContain("Confidence: 50%");
  });

  it("renders the cockpit entities", () => {
    const el = render(initialSimulationModel);
    for (const entity of ["cabin", "pilot-1", "engine-1", "alarm-1"]) {
      expect(el.textContent).toContain(entity);
    }
  });
});
