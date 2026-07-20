import { useEffect, useReducer, useState } from "react";
import { Activity, AlertTriangle, Bot, Gauge, Link, Link2Off, HelpCircle } from "lucide-react";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { KeyboardShortcutsHelp } from "./components/KeyboardShortcutsHelp";
import { SimulationEvaluation } from "./components/SimulationEvaluation";
import { SimulationSourcePanel } from "./components/SimulationSourcePanel";
import { SimulationActivityFeed } from "./components/SimulationActivityFeed";
import { SimulationWorldView } from "./components/SimulationWorldView";
import { SimulationNarrative } from "./components/SimulationNarrative";
import { SimulationProgress } from "./components/SimulationProgress";
import { findBenchmarkScenarioByPath } from "./config/scenarioCatalog";
import { KEYBOARD_SHORTCUTS } from "./config/constants";
import { simulatorClient } from "./simulatorClient";
import { initialSimulationModel, simulationReducer } from "./state/simulationReducer";
import { exponentialBackoff } from "./utils/reconnect";
import { loadPersistedSession } from "./utils/storage";
import { useI18n, type MessageKey } from "./i18n";
import type { SimulationModel } from "./types/simulation";
import packageInfo from "../package.json";

const stateLabels: Partial<Record<SimulationModel["state"], MessageKey>> = {
  connectedIdle: "connectedIdle",
  disconnected: "disconnected",
  scenarioLoading: "load",
  runCreating: "backendPending",
  running: "running",
  paused: "paused",
  ready: "ready",
  completed: "completed",
  stopped: "stopped",
  failed: "failedState"
};

export function App() {
  const { locale, setLocale, t } = useI18n();
  const persisted = loadPersistedSession();
  const [model, dispatch] = useReducer(
    simulationReducer,
    persisted
      ? { ...initialSimulationModel, approvalRequired: persisted.approvalRequired }
      : initialSimulationModel
  );
  const [showHelp, setShowHelp] = useState(false);
  const [showInsights, setShowInsights] = useState(false);
  const stateLabel = stateLabels[model.state];
  const preparingStatus = model.state === "scenarioLoading"
    ? `${t("load")}…`
    : model.state === "runCreating"
      ? `${t("backend")}: ${t("backendPending")}`
      : undefined;
  const activeScenario = findBenchmarkScenarioByPath(model.scenario?.path);

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
    simulatorClient
      .connect()
      .then(() => {
        if (!cancelled) dispatch({ type: "connected" });
      })
      .catch((error: Error) => {
        if (!cancelled) {
          dispatch({
            type: "disconnected",
            error: {
              code: "SIMULATOR_CONNECT_FAILED",
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
      await simulatorClient.connect();
      const batch = await simulatorClient.snapshot(model.lastCursor);
      if (batch.resetRequired) {
        const snapshot = await simulatorClient.simulationSnapshot();
        dispatch({ type: "snapshotReset", snapshot, cursor: batch.firstAvailableCursor - 1 });
      }
      if (batch.events.length > 0) dispatch({ type: "simulatorEvents", events: batch.events });
      if (batch.events.some((event) => event.type === "SimulationTickCommitted")) {
        const snapshot = await simulatorClient.simulationSnapshot();
        dispatch({ type: "snapshotUpdated", snapshot, cursor: batch.nextCursor });
      }
    });

    if (result.success) {
      dispatch({ type: "connected" });
    } else {
      dispatch({
        type: "disconnected",
        error: {
          code: "SIMULATOR_CONNECT_FAILED",
          message: result.error?.message ?? `${t("reconnectFailed")}: ${result.attempts}`,
          correlationId: "desktop-reconnect"
        }
      });
    }
  }

  return (
    <main className="flex h-dvh min-w-[1600px] flex-col overflow-hidden bg-zinc-950 text-zinc-100">
      <header className="grid shrink-0 grid-cols-[auto_1fr_auto] items-center gap-6 border-b border-zinc-800 px-5 py-2.5">
        <div className="flex min-w-0 items-center gap-3">
          <Activity className="h-6 w-6 shrink-0 text-cyan-300" />
          <h1 className="min-w-0 truncate text-lg font-semibold tracking-wide">{t("appName")}</h1>
          <span className="shrink-0 font-mono text-xs text-zinc-500" aria-label="build version">
            v{packageInfo.version}
          </span>
          <span className="max-w-52 shrink-0 truncate rounded border border-zinc-700 bg-zinc-900 px-2.5 py-1 text-sm text-zinc-300" title={model.scenario?.id ?? t("noScenario")}>
            {model.scenario?.id ?? t("noScenario")}
          </span>
        </div>
        <div className="w-full max-w-[420px] justify-self-center px-4">
          <SimulationProgress tick={model.tick} deadlineTick={activeScenario?.deadlineTick} state={model.state} />
        </div>
        <div className="flex shrink-0 items-center justify-self-end gap-3 text-sm text-zinc-300">
          <span className="flex shrink-0 items-center gap-1.5 whitespace-nowrap">
            {model.serviceConnected ? (
              <Link className="h-4 w-4 text-emerald-300" />
            ) : (
              <Link2Off className="h-4 w-4 text-amber-300" />
            )}
            {stateLabel ? t(stateLabel) : model.state}
          </span>
          {preparingStatus ? (
            <span aria-live="polite" className="flex shrink-0 items-center gap-1.5 whitespace-nowrap text-cyan-200">
              <span className="h-2 w-2 animate-pulse rounded-full bg-cyan-300" />
              {preparingStatus}
            </span>
          ) : null}
          {!model.serviceConnected ? (
            <button
              aria-label={t("reconnect")}
              className="shrink-0 rounded border border-zinc-700 bg-zinc-900/50 px-2.5 py-1 text-xs transition hover:bg-zinc-800 whitespace-nowrap"
              onClick={() => void reconnect()}
            >
              {t("reconnect")}
            </button>
          ) : null}
          <span className="flex shrink-0 items-center gap-1.5 whitespace-nowrap text-zinc-400">
            <Gauge className="h-4 w-4 shrink-0 text-cyan-300" />
            {t("tick")} <span className="font-mono font-medium text-zinc-200">{model.tick}</span>
          </span>
          <span className="flex max-w-48 shrink-0 items-center gap-1.5 text-zinc-400" title={model.backend}>
            <Bot className="h-4 w-4 shrink-0 text-violet-300" />
            <span className="truncate whitespace-nowrap">
              {t("modelDrive")}
              {model.backend ? ` · ${model.backend}` : ""}
            </span>
          </span>
          <div className="flex shrink-0 overflow-hidden rounded border border-zinc-700 bg-zinc-950/40" aria-label={t("language")}>
            <button
              className={`h-7 px-2 text-xs font-medium transition-colors duration-150 ${locale === "zh-CN" ? "bg-cyan-950 text-cyan-300" : "text-zinc-400 hover:bg-zinc-900/50"}`}
              onClick={() => setLocale("zh-CN")}
            >
              中
            </button>
            <button
              className={`h-7 px-2 text-xs font-medium transition-colors duration-150 ${locale === "en-US" ? "bg-cyan-950 text-cyan-300" : "text-zinc-400 hover:bg-zinc-900/50"}`}
              onClick={() => setLocale("en-US")}
            >
              EN
            </button>
          </div>
          <button
            aria-label={showInsights ? t("close") : t("evaluation")}
            aria-pressed={showInsights}
            className={`shrink-0 rounded border px-2.5 py-1 text-xs font-medium transition-all duration-150 whitespace-nowrap ${showInsights ? "border-cyan-700/60 bg-cyan-950/40 text-cyan-300 hover:bg-cyan-950/60" : "border-zinc-700 bg-zinc-900/50 hover:bg-zinc-800 text-zinc-300"}`}
            onClick={() => setShowInsights((visible) => !visible)}
          >
            {showInsights ? t("close") : t("evaluation")}
          </button>
          <button
            aria-label={t("keyboardShortcuts")}
            className="control-button h-8 w-8 shrink-0 rounded transition-colors duration-150"
            onClick={() => setShowHelp(true)}
          >
            <HelpCircle className="h-4 w-4" />
          </button>
        </div>
      </header>

      {model.error ? (
        <section className="mx-4 mt-3 flex shrink-0 items-start gap-3 border border-red-500/40 bg-red-950/30 p-3 text-sm">
          <AlertTriangle className="h-5 w-5 shrink-0 text-red-300" />
          <div className="min-w-0">
            <div className="font-medium">{model.error.code}</div>
            <div className="truncate text-red-100" title={model.error.message}>{model.error.message}</div>
          </div>
        </section>
      ) : null}

      <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-hidden p-4">
        <div className="grid min-h-0 flex-1 grid-cols-[minmax(320px,360px)_minmax(0,1fr)_minmax(360px,420px)] gap-4 overflow-hidden">
          <ErrorBoundary>
            <SimulationSourcePanel model={model} dispatch={dispatch} />
          </ErrorBoundary>
          <ErrorBoundary>
            <SimulationWorldView model={model} />
          </ErrorBoundary>
          <ErrorBoundary>
            <SimulationActivityFeed model={model} dispatch={dispatch} />
          </ErrorBoundary>
        </div>
        {showInsights ? (
          <section aria-label={t("evaluation")} className="flex h-80 shrink-0 flex-col overflow-hidden border border-zinc-800 bg-zinc-900/70">
            <div className="flex shrink-0 items-center justify-between border-b border-zinc-800 px-3 py-2">
              <div>
                <div className="text-sm font-medium">{t("evaluation")}</div>
                <div className="text-[11px] text-zinc-500">{t("dialoguePerception")}</div>
              </div>
              <button className="control-button h-7 px-2 text-xs" onClick={() => setShowInsights(false)}>
                {t("close")}
              </button>
            </div>
            <div className="grid min-h-0 flex-1 grid-cols-[minmax(360px,0.9fr)_minmax(0,1.1fr)] gap-px bg-zinc-800">
              <ErrorBoundary>
                <SimulationEvaluation model={model} />
              </ErrorBoundary>
              <ErrorBoundary>
                <SimulationNarrative model={model} />
              </ErrorBoundary>
            </div>
          </section>
        ) : null}
      </div>
      <KeyboardShortcutsHelp visible={showHelp} onClose={() => setShowHelp(false)} />
    </main>
  );
}
