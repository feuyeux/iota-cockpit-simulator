import { useEffect, useState } from "react";
import { Pause, Play, SkipForward, Square, Upload, FolderOpen } from "lucide-react";
import { APP_CONFIG, KEYBOARD_SHORTCUTS } from "../config/constants";
import { useRunner } from "../hooks/useRunner";
import { runnerClient } from "../runnerClient";
import {
  canPause,
  canStart,
  canStep,
  canStop,
  type SimulationAction
} from "../state/simulationReducer";
import type { SimulationModel } from "../types/simulation";

interface Props {
  model: SimulationModel;
  dispatch: React.Dispatch<SimulationAction>;
}

export function SimulationRunControl({ model, dispatch }: Props) {
  const { syncEvents, runCommand } = useRunner(model, dispatch);
  const [scenarioPath, setScenarioPath] = useState<string>(APP_CONFIG.DEFAULT_SCENARIO_PATH);
  const [autoStep, setAutoStep] = useState(false);
  const [stepInterval, setStepInterval] = useState(500); // ms between auto-steps

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.target instanceof HTMLElement && ["INPUT", "TEXTAREA", "SELECT"].includes(event.target.tagName)) {
        return;
      }
      if (event.key === KEYBOARD_SHORTCUTS.PAUSE && canPause(model)) {
        event.preventDefault();
        void runCommand(runnerClient.pause);
      } else if (event.key.toLowerCase() === KEYBOARD_SHORTCUTS.STEP && canStep(model)) {
        event.preventDefault();
        void runCommand(runnerClient.step);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [model, runCommand]);

  // Auto-step effect
  useEffect(() => {
    if (!autoStep || model.state !== "running") {
      return;
    }
    const intervalId = setInterval(async () => {
      if (canStep(model)) {
        await runCommand(runnerClient.step);
      } else {
        setAutoStep(false);
      }
    }, stepInterval);
    return () => clearInterval(intervalId);
  }, [autoStep, model, stepInterval, runCommand]);

  async function loadScenario(path: string) {
    dispatch({ type: "scenarioLoading" });
    let scenario;
    try {
      scenario = await runnerClient.validateScenario(path);
    } catch (error) {
      dispatch({
        type: "scenarioInvalid",
        error: {
          code: "SCENARIO_INVALID",
          message: error instanceof Error ? error.message : "scenario validation failed",
          correlationId: "desktop-scenario-validation"
        }
      });
      return;
    }
    dispatch({ type: "runCreating" });
    const runId = await runnerClient.createRun(path);
    dispatch({ type: "scenarioReady", scenario, runId });
    await syncEvents();
  }

  async function browseScenario() {
    const path = await runnerClient.openScenarioFilePicker();
    if (path) {
      setScenarioPath(path);
      await runCommand(() => loadScenario(path));
    }
  }

  async function setApprovalRequired(required: boolean) {
    if (await runCommand(() => runnerClient.setApprovalRequired(required))) {
      dispatch({ type: "approvalModeChanged", required });
    }
  }

  return (
    <section className="border border-zinc-800 bg-zinc-900/70">
      <div className="border-b border-zinc-800 px-3 py-2 text-sm font-medium">Scenario</div>
      <div className="space-y-3 p-3">
        <div className="flex gap-2">
          <button
            className="flex h-9 flex-1 items-center justify-center gap-2 border border-zinc-700 bg-zinc-800 text-sm hover:bg-zinc-700"
            onClick={() => runCommand(() => loadScenario(scenarioPath))}
          >
            <Upload className="h-4 w-4" />
            Load
          </button>
          <button
            aria-label="Browse scenario"
            className="control-button h-9 w-9"
            onClick={() => void browseScenario()}
          >
            <FolderOpen className="h-4 w-4" />
          </button>
        </div>
        <input
          aria-label="Scenario path"
          className="h-8 w-full border border-zinc-700 bg-zinc-950 px-2 text-xs text-zinc-100"
          placeholder="Scenario path"
          value={scenarioPath}
          onChange={(event) => setScenarioPath(event.target.value)}
        />
        <div className="grid grid-cols-2 gap-2">
          <button
            aria-label="Start"
            className="control-button"
            disabled={!canStart(model)}
            onClick={() => runCommand(runnerClient.start)}
          >
            <Play className="h-4 w-4" />
          </button>
          <button
            aria-label="Pause"
            className="control-button"
            disabled={!canPause(model)}
            onClick={() => runCommand(runnerClient.pause)}
          >
            <Pause className="h-4 w-4" />
          </button>
          <button
            aria-label="Step"
            className="control-button"
            disabled={!canStep(model)}
            onClick={() => runCommand(runnerClient.step)}
          >
            <SkipForward className="h-4 w-4" />
          </button>
          <button
            aria-label="Stop"
            className="control-button"
            disabled={!canStop(model)}
            onClick={() => runCommand(runnerClient.stop)}
          >
            <Square className="h-4 w-4" />
          </button>
        </div>
        <label className="flex items-center justify-between gap-3 border border-zinc-800 px-2 py-2 text-xs text-zinc-300">
          <span>Require approval</span>
          <input
            aria-label="Require approval for actions"
            checked={model.approvalRequired}
            type="checkbox"
            onChange={(event) => void setApprovalRequired(event.target.checked)}
          />
        </label>
        <label className="flex items-center justify-between gap-3 border border-zinc-800 px-2 py-2 text-xs text-zinc-300">
          <span>Auto-step</span>
          <input
            aria-label="Automatically step through simulation"
            checked={autoStep}
            type="checkbox"
            disabled={model.state !== "running"}
            onChange={(event) => setAutoStep(event.target.checked)}
          />
        </label>
        {autoStep && (
          <div className="flex items-center gap-2">
            <label className="text-xs text-zinc-400">Interval (ms):</label>
            <input
              type="number"
              min="100"
              max="5000"
              step="100"
              value={stepInterval}
              onChange={(event) => setStepInterval(Number(event.target.value))}
              className="h-7 w-20 border border-zinc-700 bg-zinc-950 px-2 text-xs text-zinc-100"
            />
          </div>
        )}
        <dl className="grid grid-cols-2 gap-2 text-xs text-zinc-300">
          <dt>Seed</dt>
          <dd className="text-right">{model.scenario?.seed ?? "-"}</dd>
          <dt>Schema</dt>
          <dd className="text-right">{model.scenario?.schemaVersion ?? "-"}</dd>
          <dt>Hash</dt>
          <dd className="truncate text-right">{model.scenario?.scenarioHash ?? "-"}</dd>
        </dl>
      </div>
    </section>
  );
}
