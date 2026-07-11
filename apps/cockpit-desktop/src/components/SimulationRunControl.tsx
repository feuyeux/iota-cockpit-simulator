import { Pause, Play, SkipForward, Square, Upload } from "lucide-react";
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
  async function syncEvents() {
    const events = await runnerClient.snapshot(model.lastCursor);
    for (const event of events) dispatch({ type: "runnerEvent", event });
  }

  async function runCommand(command: () => Promise<void>) {
    try {
      await command();
      await syncEvents();
    } catch (error) {
      dispatch({
        type: "commandRejected",
        error: {
          code: "RUNNER_COMMAND_FAILED",
          message: error instanceof Error ? error.message : "command failed",
          runId: model.runId,
          tick: model.tick,
          correlationId: "desktop-command"
        }
      });
    }
  }

  async function loadScenario() {
    dispatch({ type: "scenarioLoading" });
    const path = "scenarios/smoke-in-cockpit.yaml";
    const scenario = await runnerClient.validateScenario(path);
    await runnerClient.createRun(path);
    dispatch({ type: "scenarioReady", scenario });
    await syncEvents();
  }

  return (
    <section className="border border-zinc-800 bg-zinc-900/70">
      <div className="border-b border-zinc-800 px-3 py-2 text-sm font-medium">Scenario</div>
      <div className="space-y-3 p-3">
        <button
          className="flex h-9 w-full items-center justify-center gap-2 border border-zinc-700 bg-zinc-800 text-sm hover:bg-zinc-700"
          onClick={() => runCommand(loadScenario)}
        >
          <Upload className="h-4 w-4" />
          Load smoke scenario
        </button>
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
