import { afterEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { SimulationSourcePanel } from "./SimulationSourcePanel";
import { I18nProvider } from "../i18n";
import { simulatorClient } from "../simulatorClient";
import { initialSimulationModel } from "../state/simulationReducer";

let container: HTMLDivElement | null = null;
let root: Root | null = null;

function render() {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  act(() => {
    root!.render(
      <I18nProvider>
        <SimulationSourcePanel
          model={{ ...initialSimulationModel, state: "connectedIdle", serviceConnected: true }}
          dispatch={() => undefined}
        />
      </I18nProvider>
    );
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

describe("SimulationSourcePanel", () => {
  it("does not start auto-run when scenario validation fails", async () => {
    const validateScenario = vi.spyOn(simulatorClient, "validateScenario").mockRejectedValueOnce(new Error("invalid scenario"));
    const start = vi.spyOn(simulatorClient, "start").mockResolvedValueOnce();
    const element = render();

    await act(async () => {
      (element.querySelector('button[aria-label="一键运行"]') as HTMLButtonElement).click();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(validateScenario).toHaveBeenCalledTimes(1);
    expect(start).not.toHaveBeenCalled();
  });
});
