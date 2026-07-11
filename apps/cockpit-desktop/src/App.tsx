import { useEffect, useReducer } from "react";
import { Activity, AlertTriangle, Gauge, Link, Link2Off } from "lucide-react";
import { SimulationEvaluation } from "./components/SimulationEvaluation";
import { SimulationRunControl } from "./components/SimulationRunControl";
import { SimulationReplay } from "./components/SimulationReplay";
import { SimulationTimeline } from "./components/SimulationTimeline";
import { SimulationTrace } from "./components/SimulationTrace";
import { SimulationWorldView } from "./components/SimulationWorldView";
import { runnerClient } from "./runnerClient";
import { initialSimulationModel, simulationReducer } from "./state/simulationReducer";

export function App() {
  const [model, dispatch] = useReducer(simulationReducer, initialSimulationModel);

  useEffect(() => {
    let cancelled = false;
    dispatch({ type: "connectRequested" });
    runnerClient
      .connect()
      .then(() => {
        if (!cancelled) dispatch({ type: "connected" });
      })
      .catch((error: Error) => {
        if (!cancelled) {
          dispatch({
            type: "disconnected",
            error: {
              code: "RUNNER_CONNECT_FAILED",
              message: error.message,
              correlationId: "desktop-connect"
            }
          });
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function reconnect() {
    dispatch({ type: "connectRequested" });
    try {
      await runnerClient.connect();
      dispatch({ type: "connected" });
      const batch = await runnerClient.snapshot(model.lastCursor);
      if (batch.resetRequired) {
        const snapshot = await runnerClient.simulationSnapshot();
        dispatch({ type: "snapshotReset", snapshot, cursor: batch.firstAvailableCursor - 1 });
      }
      for (const event of batch.events) dispatch({ type: "runnerEvent", event });
    } catch (error) {
      dispatch({
        type: "disconnected",
        error: {
          code: "RUNNER_CONNECT_FAILED",
          message: error instanceof Error ? error.message : "runner reconnect failed",
          correlationId: "desktop-reconnect"
        }
      });
    }
  }

  return (
    <main className="min-h-screen bg-zinc-950 text-zinc-100">
      <header className="flex min-h-14 items-center justify-between border-b border-zinc-800 px-4">
        <div className="flex items-center gap-3">
          <Activity className="h-5 w-5 text-cyan-300" />
          <h1 className="text-base font-semibold">Cockpit Simulation</h1>
          <span className="rounded border border-zinc-700 px-2 py-1 text-xs text-zinc-300">
            {model.scenario?.id ?? "no scenario"}
          </span>
        </div>
        <div className="flex items-center gap-4 text-sm text-zinc-300">
          <span className="flex items-center gap-2">
            {model.serviceConnected ? (
              <Link className="h-4 w-4 text-emerald-300" />
            ) : (
              <Link2Off className="h-4 w-4 text-amber-300" />
            )}
            {model.state}
          </span>
          {!model.serviceConnected ? (
            <button
              aria-label="Reconnect runner"
              className="border border-zinc-700 px-2 py-1 text-xs hover:bg-zinc-800"
              onClick={() => void reconnect()}
            >
              Reconnect
            </button>
          ) : null}
          <span className="flex items-center gap-2">
            <Gauge className="h-4 w-4 text-cyan-300" />
            tick {model.tick} / {model.simTimeMs}ms
          </span>
        </div>
      </header>

      {model.error ? (
        <section className="mx-4 mt-4 flex items-start gap-3 border border-red-500/40 bg-red-950/30 p-3 text-sm">
          <AlertTriangle className="h-5 w-5 text-red-300" />
          <div>
            <div className="font-medium">{model.error.code}</div>
            <div className="text-red-100">{model.error.message}</div>
          </div>
        </section>
      ) : null}

      <div className="grid gap-4 p-4 xl:grid-cols-[280px_minmax(460px,1fr)_360px]">
        <SimulationRunControl model={model} dispatch={dispatch} />
        <SimulationWorldView model={model} />
        <SimulationEvaluation model={model} />
      </div>
      <div className="grid gap-4 px-4 pb-4 xl:grid-cols-2">
        <SimulationTimeline model={model} />
        <SimulationTrace model={model} dispatch={dispatch} />
        <SimulationReplay model={model} dispatch={dispatch} />
      </div>
    </main>
  );
}
