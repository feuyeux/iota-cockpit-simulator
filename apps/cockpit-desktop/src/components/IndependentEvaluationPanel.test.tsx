import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../i18n";
import { initialSimulationModel } from "../state/simulationReducer";
import type { EvaluationReportRecord, SimulationModel } from "../types/simulation";
import { IndependentEvaluationPanel } from "./IndependentEvaluationPanel";

const report: EvaluationReportRecord = {
  id: "1000-run-1",
  createdAtMs: 1000,
  runId: "run-1",
  scenarioId: "smoke-in-cockpit",
  report: {
    schemaVersion: 1,
    verdict: "pass",
    rubricId: "smoke-private",
    rubricVersion: "1",
    rubricHash: "sha256:rubric",
    inputHash: "sha256:input",
    schemaHash: "sha256:schema",
    deterministicResults: [{
      ruleId: "shutdown-before-spread",
      deadlineTick: 30,
      verdict: "pass",
      result: { passed: true, score: 1, evidenceEventIds: ["event-1"], firstFailureTick: null, explanation: "passed" }
    }],
    evidence: [{ tick: 6, entityId: "engine-1", eventId: "event-1", kind: "EngineShutdown" }],
    judges: [{
      verdict: "pass",
      confidence: 0.95,
      explanation: "supported",
      evidence: [{ tick: 6, eventId: "event-1", kind: "EngineShutdown" }],
      provenance: {
        judgeId: "judge-a",
        model: "model-a",
        promptHash: "sha256:prompt-a",
        rubricHash: "sha256:rubric",
        schemaHash: "sha256:schema"
      }
    }, {
      verdict: "pass",
      confidence: 0.9,
      explanation: "supported",
      evidence: [{ tick: 6, eventId: "event-1", kind: "EngineShutdown" }],
      provenance: {
        judgeId: "judge-b",
        model: "model-b",
        promptHash: "sha256:prompt-b",
        rubricHash: "sha256:rubric",
        schemaHash: "sha256:schema"
      }
    }],
    judgeDisagreement: false,
    releaseGatePassed: true,
    explanation: "all gates passed"
  }
};

function completedModel(runId: string): SimulationModel {
  return {
    ...initialSimulationModel,
    state: "completed",
    tick: 80,
    runId,
    scenario: {
      id: "smoke-in-cockpit",
      path: "scenarios/smoke-in-cockpit.yaml",
      schemaVersion: 1,
      scenarioHash: "hash",
      seed: 42,
      agentId: "agent"
    }
  };
}


const mocks = vi.hoisted(() => ({
  evaluateRun: vi.fn(),
  listEvaluationReports: vi.fn(async () => [] as EvaluationReportRecord[])
}));
vi.mock("../simulatorClient", () => ({
  simulatorClient: {
    evaluateRun: mocks.evaluateRun,
    listEvaluationReports: mocks.listEvaluationReports
  }
}));

let container: HTMLDivElement | undefined;
let root: Root | undefined;

afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
  root = undefined;
  container = undefined;
  vi.clearAllMocks();
});

describe("IndependentEvaluationPanel", () => {
  it("runs the independent evaluator and renders evidence, Judges, and history", async () => {
    mocks.evaluateRun.mockResolvedValue(report);
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => {
      root!.render(
        <I18nProvider>
          <IndependentEvaluationPanel model={{
            ...initialSimulationModel,
            state: "completed",
            tick: 80,
            runId: "run-1",
            scenario: {
              id: "smoke-in-cockpit",
              path: "scenarios/smoke-in-cockpit.yaml",
              schemaVersion: 1,
              scenarioHash: "hash",
              seed: 42,
              agentId: "agent"
            }
          }} />
        </I18nProvider>
      );
    });

    const button = Array.from(container.querySelectorAll("button"))
      .find((item) => item.textContent?.includes("一键独立评测"));
    expect(button).toBeDefined();
    await act(async () => {
      button!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(mocks.evaluateRun).toHaveBeenCalledWith("run-1", "smoke-in-cockpit");
    expect(container.textContent).toContain("pass");
    expect(container.textContent).toContain("shutdown-before-spread");
    expect(container.textContent).toContain("EngineShutdown");
    expect(container.textContent).toContain("双 Judge 一致");
    expect(container.textContent).toContain("judge-a · model-a");
    expect(container.textContent).toContain("报告历史");
  });

  it("clears a selected report when the active run has no matching history", async () => {
    mocks.listEvaluationReports
      .mockResolvedValueOnce([report])
      .mockResolvedValueOnce([]);
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);

    await act(async () => {
      root!.render(
        <I18nProvider>
          <IndependentEvaluationPanel model={completedModel("run-1")} />
        </I18nProvider>
      );
    });
    expect(container.textContent).toContain("pass");
    expect(container.textContent).toContain("run-1");

    await act(async () => {
      root!.render(
        <I18nProvider>
          <IndependentEvaluationPanel model={completedModel("run-2")} />
        </I18nProvider>
      );
    });

    expect(container.textContent).not.toContain("pass");
    expect(container.textContent).not.toContain("run-1");
  });
});
