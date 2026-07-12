import { useEffect, useReducer, useState } from "react";
import { Activity, AlertTriangle, Gauge, Link, Link2Off, HelpCircle } from "lucide-react";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { KeyboardShortcutsHelp } from "./components/KeyboardShortcutsHelp";
import { SimulationEvaluation } from "./components/SimulationEvaluation";
import { SimulationRunControl } from "./components/SimulationRunControl";
import { SimulationReplay } from "./components/SimulationReplay";
import { SimulationTimeline } from "./components/SimulationTimeline";
import { SimulationTrace } from "./components/SimulationTrace";
import { SimulationWorldView } from "./components/SimulationWorldView";
import { KEYBOARD_SHORTCUTS } from "./config/constants";
import { runnerClient } from "./runnerClient";
import { initialSimulationModel, simulationReducer } from "./state/simulationReducer";
import { exponentialBackoff } from "./utils/reconnect";
import { loadPersistedSession } from "./utils/storage";

export function App() {
  const persisted = loadPersistedSession();
  const [model, dispatch] = useReducer(
    simulationReducer,
    persisted
      ? { ...initialSimulationModel, approvalRequired: persisted.approvalRequired }
      : initialSimulationModel
  );
  const [showHelp, setShowHelp] = useState(false);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.target instanceof HTMLElement && ["INPUT", "TEXTAREA", "SELECT"].includes(event.target.tagName)) {
        return;
      }
      if (event.key === KEYBOARD_SHORTCUTS.HELP) {
        event.preventDefault();
        setShowHelp(true);
      } else if (event.key === "Escape") {
        setShowHelp(false);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

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
    const result = await exponentialBackoff(async () => {
      await runnerClient.connect();
      const batch = await runnerClient.snapshot(model.lastCursor);
      if (batch.resetRequired) {
        const snapshot = await runnerClient.simulationSnapshot();
        dispatch({ type: "snapshotReset", snapshot, cursor: batch.firstAvailableCursor - 1 });
      }
      for (const event of batch.events) dispatch({ type: "runnerEvent", event });
    });

    if (result.success) {
      dispatch({ type: "connected" });
    } else {
      dispatch({
        type: "disconnected",
        error: {
          code: "RUNNER_CONNECT_FAILED",
          message: result.error?.message ?? `reconnect failed after ${result.attempts} attempts`,
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
          <button
            aria-label="Keyboard shortcuts"
            className="control-button h-7 w-7"
            onClick={() => setShowHelp(true)}
          >
            <HelpCircle className="h-4 w-4" />
          </button>
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
        <ErrorBoundary>
          <SimulationRunControl model={model} dispatch={dispatch} />
        </ErrorBoundary>
        <ErrorBoundary>
          <SimulationWorldView model={model} />
        </ErrorBoundary>
        <ErrorBoundary>
          <SimulationEvaluation model={model} />
        </ErrorBoundary>
      </div>
      <div className="grid gap-4 px-4 pb-4 xl:grid-cols-2">
        <ErrorBoundary>
          <SimulationTimeline model={model} />
        </ErrorBoundary>
        <ErrorBoundary>
          <SimulationTrace model={model} dispatch={dispatch} />
        </ErrorBoundary>
        <ErrorBoundary>
          <SimulationReplay model={model} dispatch={dispatch} />
        </ErrorBoundary>
      </div>
      <KeyboardShortcutsHelp visible={showHelp} onClose={() => setShowHelp(false)} />
    </main>
  );
}
