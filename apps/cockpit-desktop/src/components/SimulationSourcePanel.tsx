import { useEffect, useRef, useState } from "react";
import {
  FolderOpen,
  FastForward,
  GitCompareArrows,
  Pause,
  Play,
  SkipForward,
  Square,
  Upload
} from "lucide-react";
import { APP_CONFIG, KEYBOARD_SHORTCUTS } from "../config/constants";
import { useSimulator } from "../hooks/useSimulator";
import { simulatorClient } from "../simulatorClient";
import { describeError } from "../utils/describeError";
import {
  canPause,
  canStart,
  canStep,
  canStop,
  type SimulationAction
} from "../state/simulationReducer";
import type { RecordingDiff, SimulationModel } from "../types/simulation";
import { BENCHMARK_SCENARIOS, COCKPIT_DOMAINS, localize } from "../config/scenarioCatalog";
import { useI18n } from "../i18n";

interface Props {
  model: SimulationModel;
  dispatch: React.Dispatch<SimulationAction>;
}

type SourceMode = "live" | "replay";

/// Consolidates what used to be two separate panels (Scenario/RunControl and
/// Replay) into a single "Run Source" panel with two tabs. Both panels were
/// answering the same underlying question - "what data is driving the world
/// view right now" - so splitting them made the product harder to learn.
/// Live = drive the world from a running scenario via the backend (hermes)
/// human-decision loop. Replay = drive it from a recorded run instead. Only
/// one is ever "active" at a time, so a tabbed layout keeps controls close
/// without permanently doubling vertical space.
///
/// There used to be a second "rule demo" drive mode backed by a local
/// deterministic RuleAgent (no real model call). It has been removed from
/// this desktop surface: every human decision is now always driven by a
/// real backend turn (hermes via iota-core ACP), matching the product's
/// actual requirement of evaluating simulations through the backend rather
/// than a scripted stand-in. The Rust-side RuleAgent/rule IPC commands still
/// exist and remain load-bearing for the Rust contract/integration test
/// suite and the offline CLI demo path - only this desktop UI's toggle and
/// its dedicated Tauri bindings were removed.
function reportFailure(
  dispatch: React.Dispatch<SimulationAction>,
  model: SimulationModel,
  error: unknown,
  fallbackMessage: string
) {
  dispatch({
    type: "commandRejected",
    error: {
      code: "SIMULATOR_COMMAND_FAILED",
      message: describeError(error, fallbackMessage),
      correlationId: "desktop-source-panel",
      runId: model.runId,
      tick: model.tick
    }
  });
}

function DiffSummary({ report }: { report: RecordingDiff }) {
  const { t } = useI18n();
  const divergence = report.firstDivergence;
  return (
    <div className="space-y-1 border-t border-zinc-800 pt-2 text-xs text-zinc-300">
      <div className={report.equivalent ? "text-emerald-300" : "text-amber-300"}>
        {report.equivalent ? t("equivalentRecordings") : t("recordingDivergence")}
      </div>
      {divergence ? <div>{t("firstDivergence")}: {divergence.tick}</div> : null}
      <div>
        {t("source")} {report.sourceMetrics.ticks} {t("ticksUnit")} / {t("candidate")} {report.candidateMetrics.ticks} {t("ticksUnit")}
      </div>
    </div>
  );
}

export function SimulationSourcePanel({ model, dispatch }: Props) {
  const { locale, t } = useI18n();
  const { syncEvents, runCommand } = useSimulator(model, dispatch);
  const [mode, setMode] = useState<SourceMode>("live");
  // Hermes cold-starts its ACP session and tool surface before the first
  // prompt, which regularly exceeds a 20-second end-to-end budget.
  const [modelTimeoutMs, setModelTimeoutMs] = useState(60_000);
  const [liveTurnInFlight, setLiveTurnInFlight] = useState(false);
  const [autoRunInFlight, setAutoRunInFlight] = useState(false);
  const autoRunCancelled = useRef(false);
  const [scenarioLoadInFlight, setScenarioLoadInFlight] = useState(false);
  const scenarioLoadLock = useRef(false);
  const [scenarioPath, setScenarioPath] = useState<string>(APP_CONFIG.DEFAULT_SCENARIO_PATH);
  const [recordingPath, setRecordingPath] = useState("");
  const [candidatePath, setCandidatePath] = useState("");
  const selectedScenario = BENCHMARK_SCENARIOS.find((scenario) => scenario.path === scenarioPath);
  // The timeout only takes effect on the next Load, so lock the input while a
  // run is actually in flight to avoid a control that silently has no effect
  // on the active run.
  const timeoutLocked = canStop(model) || model.state === "scenarioLoading" || model.state === "runCreating";

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.target instanceof HTMLElement && ["INPUT", "TEXTAREA", "SELECT"].includes(event.target.tagName)) {
        return;
      }
      if (event.key === KEYBOARD_SHORTCUTS.PAUSE && canPause(model) && !liveTurnInFlight && !autoRunInFlight) {
        event.preventDefault();
        void runCommand(simulatorClient.pause);
      } else if (
        event.key.toLowerCase() === KEYBOARD_SHORTCUTS.STEP
        && canStep(model)
        && !liveTurnInFlight
        && !autoRunInFlight
      ) {
        event.preventDefault();
        void stepOnce();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [model, runCommand, liveTurnInFlight, autoRunInFlight]);

  async function loadScenario(path: string): Promise<boolean> {
    // State updates do not take effect until React's next render. Keep a ref
    // lock as well so rapid clicks cannot enqueue multiple expensive Hermes
    // warm-ups before the Load button becomes disabled.
    if (scenarioLoadLock.current) return false;
    scenarioLoadLock.current = true;
    setScenarioLoadInFlight(true);
    dispatch({ type: "scenarioLoading" });
    try {
      let scenario;
      try {
        scenario = await simulatorClient.validateScenario(path);
      } catch (error) {
        dispatch({
          type: "scenarioInvalid",
          error: {
            code: "SCENARIO_INVALID",
            message: describeError(error, t("scenarioValidationFailed")),
            correlationId: "desktop-scenario-validation"
          }
        });
        return false;
      }
      dispatch({ type: "runCreating" });
      const live = await simulatorClient.createLiveRun(path, modelTimeoutMs);
      dispatch({
        type: "scenarioReady",
        scenario,
        runId: live.runId,
        backend: live.backend
      });
      return true;
    } finally {
      scenarioLoadLock.current = false;
      setScenarioLoadInFlight(false);
    }
  }

  async function stepOnce() {
    if (liveTurnInFlight || autoRunInFlight) return;
    setLiveTurnInFlight(true);
    try {
      await runCommand(simulatorClient.stepLive);
    } finally {
      setLiveTurnInFlight(false);
    }
  }

  async function autoRunScenario() {
    if (autoRunInFlight || liveTurnInFlight || scenarioLoadInFlight) return;
    autoRunCancelled.current = false;
    setAutoRunInFlight(true);
    let cursor = model.lastCursor;
    try {
      const loaded = await loadScenario(scenarioPath);
      if (!loaded) return;
      await syncEvents();
      if (!(await runCommand(simulatorClient.start))) return;

      // The load/start commands have already synchronized their events. Keep
      // a local cursor for the loop because React state is intentionally not
      // updated synchronously between ACP-backed ticks.
      const initialBatch = await simulatorClient.snapshot(model.lastCursor);
      cursor = initialBatch.nextCursor;
      const maxTicks = selectedScenario?.deadlineTick ?? 20;

      for (let index = 0; index < maxTicks && !autoRunCancelled.current; index += 1) {
        const result = await simulatorClient.stepLive();
        const status = typeof result === "object" && result !== null && "status" in result
          ? String((result as { status?: unknown }).status)
          : "";
        const batch = await simulatorClient.snapshot(cursor);
        if (batch.resetRequired) {
          const snapshot = await simulatorClient.simulationSnapshot();
          dispatch({ type: "snapshotReset", snapshot, cursor: batch.firstAvailableCursor - 1 });
          cursor = batch.firstAvailableCursor - 1;
        }
        if (batch.events.length > 0) dispatch({ type: "simulatorEvents", events: batch.events });
        if (batch.events.some((event) => event.type === "SimulationTickCommitted")) {
          const snapshot = await simulatorClient.simulationSnapshot();
          dispatch({ type: "snapshotUpdated", snapshot, cursor: batch.nextCursor });
        }
        cursor = batch.nextCursor;
        if (["completed", "stopped", "failed"].includes(status)) break;
        await new Promise((resolve) => window.setTimeout(resolve, APP_CONFIG.AUTO_RUN_EVENT_POLL_INTERVAL_MS));
      }
    } catch (error) {
      try {
        const batch = await simulatorClient.snapshot(cursor);
        if (batch.resetRequired) {
          const snapshot = await simulatorClient.simulationSnapshot();
          dispatch({ type: "snapshotReset", snapshot, cursor: batch.firstAvailableCursor - 1 });
        }
        if (batch.events.length > 0) dispatch({ type: "simulatorEvents", events: batch.events });
        if (batch.events.some((event) => event.type === "SimulationTickCommitted")) {
          const snapshot = await simulatorClient.simulationSnapshot();
          dispatch({ type: "snapshotUpdated", snapshot, cursor: batch.nextCursor });
        }
        if (batch.events.some((event) => event.type === "SimulationError")) return;
      } catch {
        // Fall back to the command error when the Simulator event stream is unavailable.
      }
      reportFailure(dispatch, model, error, t("commandFailed"));
    } finally {
      setAutoRunInFlight(false);
    }
  }

  async function stopRun() {
    autoRunCancelled.current = true;
    try {
      await simulatorClient.cancelLiveTurn();
    } catch (error) {
      reportFailure(dispatch, model, error, t("commandFailed"));
      return;
    }
    await runCommand(simulatorClient.stop);
  }

  async function browseScenario() {
    const path = await simulatorClient.openScenarioFilePicker();
    if (path) {
      setScenarioPath(path);
      await runCommand(() => loadScenario(path));
    }
  }

  async function replay() {
    if (!model.scenario || !recordingPath) return;
    try {
      await simulatorClient.startReplay(model.scenario.path, recordingPath);
      await syncEvents();
    } catch (error) {
      reportFailure(dispatch, model, error, t("commandFailed"));
    }
  }

  async function compare() {
    if (!recordingPath || !candidatePath) return;
    try {
      const report = await simulatorClient.diffRecordings(recordingPath, candidatePath);
      dispatch({ type: "replayDiffUpdated", report });
    } catch (error) {
      reportFailure(dispatch, model, error, t("commandFailed"));
    }
  }

  async function browseRecording(target: "source" | "candidate") {
    const path = await simulatorClient.openRecordingFilePicker();
    if (path) {
      if (target === "source") setRecordingPath(path);
      else setCandidatePath(path);
    }
  }

  return (
    <section className="flex min-h-0 min-w-0 flex-col overflow-hidden border border-zinc-800 bg-zinc-900/70">
      <div className="flex shrink-0 border-b border-zinc-800 text-sm font-medium">
        <button
          className={`flex-1 px-3 py-2 ${mode === "live" ? "border-b-2 border-cyan-400 text-cyan-200" : "text-zinc-500 hover:text-zinc-300"}`}
          onClick={() => setMode("live")}
        >
          {t("liveRun")}
        </button>
        <button
          className={`flex-1 px-3 py-2 ${mode === "replay" ? "border-b-2 border-cyan-400 text-cyan-200" : "text-zinc-500 hover:text-zinc-300"}`}
          onClick={() => setMode("replay")}
        >
          {t("replay")}
        </button>
      </div>

      {mode === "live" ? (
        <div className="min-h-0 flex-1 space-y-3 overflow-y-auto p-3">
          <div className="border border-cyan-700/50 bg-cyan-950/20 p-2.5 text-xs">
            <p className="leading-4 text-cyan-100/80">{t("modelDriveNotice")}</p>
            <div className="mt-2 flex items-center justify-between gap-2 text-[10px] text-zinc-500">
              <span>{t("backend")}</span>
              <code className="truncate text-zinc-300">{model.backend ?? t("backendPending")}</code>
            </div>
            <label className="mt-2 flex items-center justify-between gap-2 text-[10px] text-zinc-500">
              <span>{t("modelTimeout")}</span>
              <input
                className="h-7 w-24 border border-zinc-700 bg-zinc-950 px-2 text-right text-xs text-zinc-100 disabled:opacity-40"
                disabled={timeoutLocked}
                min={2_000}
                max={120_000}
                step={1_000}
                type="number"
                value={modelTimeoutMs}
                onChange={(event) => setModelTimeoutMs(Number(event.target.value))}
              />
            </label>
          </div>
          <div>
            <label className="mb-1 block text-xs font-medium text-zinc-300" htmlFor="benchmark-scenario">
              {t("stepChooseScenario")}
            </label>
            <select
              id="benchmark-scenario"
              className="h-9 w-full border border-zinc-700 bg-zinc-950 px-2 text-xs text-zinc-100"
              title={selectedScenario ? localize(selectedScenario.title, locale) : t("customScenario")}
              value={selectedScenario?.id ?? "custom"}
              onChange={(event) => {
                const scenario = BENCHMARK_SCENARIOS.find((item) => item.id === event.target.value);
                if (scenario) setScenarioPath(scenario.path);
              }}
            >
              {BENCHMARK_SCENARIOS.map((scenario, index) => (
                <option key={scenario.id} value={scenario.id}>
                  {String(index + 1).padStart(2, "0")} · {localize(scenario.title, locale)}
                </option>
              ))}
              {!selectedScenario ? <option value="custom">{t("customScenario")}</option> : null}
            </select>
            <p className="mt-1 text-[10px] leading-4 text-zinc-500">{t("chooseScenario")}</p>
            <div className="mt-3 space-y-2">
              <button
                className="flex w-full items-center gap-3 border border-cyan-600 bg-cyan-950/60 px-3 py-2.5 text-left text-cyan-50 transition hover:bg-cyan-900/60 disabled:opacity-40"
                disabled={liveTurnInFlight || autoRunInFlight || scenarioLoadInFlight}
                onClick={() => runCommand(() => loadScenario(scenarioPath))}
              >
                <Upload className="h-4 w-4 shrink-0 text-cyan-300" />
                <span className="min-w-0"><span className="block text-sm font-medium">{t("stepLoadScenario")}</span><span className="block text-[10px] text-cyan-200/70">{t("stepLoadScenarioDetail")}</span></span>
              </button>
              <button
                aria-label={t("autoRun")}
                className="flex w-full items-center gap-3 border border-emerald-500 bg-emerald-950/50 px-3 py-2.5 text-left text-emerald-50 transition hover:bg-emerald-900/50 disabled:opacity-40"
                disabled={!model.serviceConnected || liveTurnInFlight || autoRunInFlight || scenarioLoadInFlight}
                onClick={() => void autoRunScenario()}
              >
                <FastForward className="h-4 w-4 shrink-0 text-emerald-300" />
                <span className="min-w-0"><span className="block text-sm font-medium">{t("stepRunAndEvaluate")}</span><span className="block text-[10px] text-emerald-200/70">{t("stepRunAndEvaluateDetail")}</span></span>
              </button>
            </div>
            <div className="mt-3 border-t border-zinc-800 pt-2">
              <div className="mb-2 text-[10px] font-medium uppercase tracking-wide text-zinc-500">{t("useForCloseInspection")}</div>
              <div className="grid grid-cols-4 gap-2">
                <button aria-label={t("start")} className="control-button h-9 flex-col gap-0.5 text-[10px]" disabled={!canStart(model)} onClick={() => runCommand(simulatorClient.start)}><Play className="h-3.5 w-3.5" />{t("start")}</button>
                <button aria-label={t("step")} className="control-button h-9 flex-col gap-0.5 text-[10px]" disabled={!canStep(model) || liveTurnInFlight} onClick={() => void stepOnce()}><SkipForward className="h-3.5 w-3.5" />{t("step")}</button>
                <button aria-label={t("pause")} className="control-button h-9 flex-col gap-0.5 text-[10px]" disabled={!canPause(model) || liveTurnInFlight} onClick={() => runCommand(simulatorClient.pause)}><Pause className="h-3.5 w-3.5" />{t("pause")}</button>
                <button aria-label={t("stop")} className="control-button h-9 flex-col gap-0.5 text-[10px]" disabled={!model.serviceConnected || (!canStop(model) && !liveTurnInFlight && !autoRunInFlight)} onClick={() => void stopRun()}><Square className="h-3.5 w-3.5" />{t("stop")}</button>
              </div>
            </div>
          </div>
          {selectedScenario ? (
            <div className="max-h-64 space-y-2 overflow-y-auto border border-zinc-800 bg-zinc-950/60 p-2.5 text-xs [overflow-wrap:anywhere]">
              <div className="flex items-start justify-between gap-2">
                <div>
                  <div className="text-[10px] uppercase text-zinc-500">{t("domain")}</div>
                  <div className="font-medium text-cyan-200">{localize(selectedScenario.domain, locale)}</div>
                </div>
                <div className="shrink-0 text-right text-[10px] text-zinc-500">
                  {selectedScenario.occupants} {t("participants")} · {selectedScenario.systems} {t("systems")}
                </div>
              </div>
              <div>
                <span className="text-zinc-500">{t("objective")}: </span>
                <span className="text-zinc-200">{localize(selectedScenario.objective, locale)}</span>
              </div>
              <div>
                <span className="text-zinc-500">{t("risk")}: </span>
                <span className="text-amber-200">{localize(selectedScenario.risk, locale)}</span>
              </div>
              <div>
                <div className="mb-1 text-zinc-500">{t("trigger")}</div>
                <p className="leading-4 text-zinc-300">{localize(selectedScenario.trigger, locale)}</p>
              </div>
              <div>
                <div className="mb-1 text-zinc-500">{t("coverage")}</div>
                <div className="flex flex-wrap gap-1">
                  {selectedScenario.coverage.map((item) => (
                    <span key={localize(item, locale)} className="border border-zinc-700 px-1.5 py-0.5 text-[10px] text-zinc-300">
                      {localize(item, locale)}
                    </span>
                  ))}
                </div>
              </div>
              <div className="border-t border-zinc-800 pt-2">
                <div className="mb-1.5 text-zinc-500">{t("actionContract")}</div>
                <dl className="grid grid-cols-[78px_minmax(0,1fr)] gap-x-2 gap-y-1 text-[10px]">
                  <dt className="text-zinc-600">{t("capability")}</dt>
                  <dd className="break-all font-mono text-cyan-200">{selectedScenario.capability}</dd>
                  <dt className="text-zinc-600">{t("command")}</dt>
                  <dd className="break-all font-mono text-emerald-200">{selectedScenario.command}</dd>
                  <dt className="text-zinc-600">{t("target")}</dt>
                  <dd className="break-all font-mono text-zinc-300">{selectedScenario.target}</dd>
                  <dt className="text-zinc-600">{t("evidenceEvent")}</dt>
                  <dd className="break-all font-mono text-violet-200">{selectedScenario.evidenceEvent}</dd>
                  <dt className="text-zinc-600">{t("deadlineTick")}</dt>
                  <dd className="font-mono text-amber-200">t{selectedScenario.deadlineTick}</dd>
                </dl>
              </div>
              <div className="border-t border-zinc-800 pt-2">
                <div className="mb-1 flex items-center justify-between gap-2">
                  <span className="text-zinc-500">{t("domainCoverage")}</span>
                  <span className="text-[10px] text-emerald-300">{t("fullDomainCoverage")}</span>
                </div>
                <div className="mb-1 text-[10px] text-zinc-600">
                  {t("scenarioContribution")}: {selectedScenario.domains.length} / {COCKPIT_DOMAINS.length}
                </div>
                <div className="grid grid-cols-2 gap-1">
                  {COCKPIT_DOMAINS.map((domain) => {
                    const active = selectedScenario.domains.includes(domain.id);
                    const scenarioCount = BENCHMARK_SCENARIOS.filter((scenario) => scenario.domains.includes(domain.id)).length;
                    return (
                      <div
                        key={domain.id}
                        className={`flex min-h-7 items-center justify-between gap-1 border px-1.5 py-1 text-[9px] leading-3 ${active ? "border-cyan-700/70 bg-cyan-950/30 text-cyan-100" : "border-zinc-800 text-zinc-600"}`}
                        title={`${scenarioCount} ${t("scenariosUnit")}`}
                      >
                        <span>{localize(domain.label, locale)}</span>
                        <span className={active ? "text-cyan-300" : "text-zinc-700"}>{scenarioCount}</span>
                      </div>
                    );
                  })}
                </div>
              </div>
            </div>
          ) : null}
          <div className="flex gap-2">
            <input
              aria-label={t("scenarioPath")}
              className="h-8 min-w-0 flex-1 border border-zinc-700 bg-zinc-950 px-2 text-xs text-zinc-100"
              placeholder={t("scenarioPath")}
              value={scenarioPath}
              onChange={(event) => setScenarioPath(event.target.value)}
            />
            <button
              aria-label={t("browseScenario")}
              className="control-button h-8 w-8"
              disabled={scenarioLoadInFlight}
              onClick={() => void browseScenario()}
            >
              <FolderOpen className="h-3 w-3" />
            </button>
          </div>
          {liveTurnInFlight ? (
            <div className="flex items-center gap-2 border border-violet-700/50 bg-violet-950/20 px-2 py-1.5 text-[10px] text-violet-200">
              <span className="h-2 w-2 animate-pulse rounded-full bg-violet-300" />
              {t("turnInFlight")}
            </div>
          ) : null}
          {autoRunInFlight ? (
            <div className="flex items-center gap-2 border border-cyan-700/50 bg-cyan-950/20 px-2 py-1.5 text-[10px] text-cyan-200">
              <FastForward className="h-3 w-3 animate-pulse" />
              {t("autoRun")}
            </div>
          ) : null}
          <dl className="grid grid-cols-2 gap-2 text-xs text-zinc-300">
            <dt>{t("seed")}</dt>
            <dd className="text-right">{model.scenario?.seed ?? "-"}</dd>
            <dt>{t("schema")}</dt>
            <dd className="text-right">{model.scenario?.schemaVersion ?? "-"}</dd>
            <dt>{t("hash")}</dt>
            <dd className="truncate text-right">{model.scenario?.scenarioHash ?? "-"}</dd>
          </dl>
        </div>
      ) : (
        <div className="min-h-0 flex-1 space-y-2 overflow-y-auto p-3">
          <div className="flex gap-2">
            <input
              aria-label={t("recordingPath")}
              className="h-8 flex-1 border border-zinc-700 bg-zinc-950 px-2 text-xs text-zinc-100"
              placeholder={t("recordingPath")}
              value={recordingPath}
              onChange={(event) => setRecordingPath(event.target.value)}
            />
            <button
              aria-label={t("browseRecording")}
              className="control-button h-8 w-8"
              onClick={() => void browseRecording("source")}
            >
              <FolderOpen className="h-3 w-3" />
            </button>
          </div>
          <div className="grid grid-cols-2 gap-2">
            <button
              aria-label={t("replayRecording")}
              className="control-button"
              disabled={!model.scenario || !recordingPath}
              onClick={() => void replay()}
              title={t("replayRecording")}
            >
              <Play className="h-4 w-4" />
            </button>
            <button
              aria-label={t("compareRecordings")}
              className="control-button"
              disabled={!recordingPath || !candidatePath}
              onClick={() => void compare()}
              title={t("compareRecordings")}
            >
              <GitCompareArrows className="h-4 w-4" />
            </button>
          </div>
          <div className="flex gap-2">
            <input
              aria-label={t("comparisonPath")}
              className="h-8 flex-1 border border-zinc-700 bg-zinc-950 px-2 text-xs text-zinc-100"
              placeholder={t("comparisonPath")}
              value={candidatePath}
              onChange={(event) => setCandidatePath(event.target.value)}
            />
            <button
              aria-label={t("browseRecording")}
              className="control-button h-8 w-8"
              onClick={() => void browseRecording("candidate")}
            >
              <FolderOpen className="h-3 w-3" />
            </button>
          </div>
          {model.replayDiff ? <DiffSummary report={model.replayDiff} /> : null}
          {!model.scenario && (
            <p className="text-[11px] text-zinc-500">
              {t("loadScenarioFirst")}
            </p>
          )}
        </div>
      )}
    </section>
  );
}
