import { useMemo, useState } from "react";
import { Ban, Bot, Check, ChevronLeft, ChevronRight, Download, Wrench, X, Zap } from "lucide-react";
import { APP_CONFIG } from "../config/constants";
import { useSimulator } from "../hooks/useSimulator";
import {
  exportActionResultsAsJSON,
  exportEventsAsCSV,
  exportEventsAsJSON,
  exportTracesAsCSV,
  exportTracesAsJSON
} from "../utils/export";
import { simulatorClient } from "../simulatorClient";
import type { SimulationAction } from "../state/simulationReducer";
import type {
  ActionResult,
  HumanTurnTrace,
  SimulationEvent,
  SimulationModel,
  ToolCallTrace
} from "../types/simulation";
import { useI18n } from "../i18n";
import { describeError } from "../utils/describeError";
import {
  actionStatusLabel,
  commandLabel,
  eventDescription,
  eventLabel
} from "../utils/domainPresentation";

interface Props {
  model: SimulationModel;
  dispatch: React.Dispatch<SimulationAction>;
}

/// Unified, chronologically-merged replacement for what used to be two
/// separate panels: Timeline (raw SimulationEvents) and Agent Trace (tool
/// calls + action results). Both were answering the same question - "what
/// just happened in this run" - from different angles, which made it unclear
/// where to look first. Merging them into one feed, ordered by tick with a
/// severity/emphasis cue, gives a single place to watch and makes pending
/// approvals impossible to miss since they're inline instead of in a
/// separately-scrolled panel.
type FeedItem =
  | { kind: "event"; tick: number; sequence: number; data: SimulationEvent }
  | { kind: "toolCall"; tick: number; sequence: number; data: ToolCallTrace }
  | { kind: "humanTurn"; tick: number; sequence: number; data: HumanTurnTrace }
  | { kind: "actionResult"; tick: number; sequence: number; data: ActionResult };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isPendingApproval(result: unknown): boolean {
  if (!isRecord(result) || !isRecord(result.result)) return false;
  return result.result.status === "pendingApproval";
}

function pendingRequestId(trace: ToolCallTrace, actionResults: ActionResult[]): string | undefined {
  if (trace.toolName !== "simulation.request_action" || !isPendingApproval(trace.result)) return undefined;
  return actionResults.some((result) => result.request.requestId === trace.callId) ? undefined : trace.callId;
}

function buildFeed(model: SimulationModel): FeedItem[] {
  const items: FeedItem[] = [
    ...model.events.map((event) => ({ kind: "event" as const, tick: event.tick, sequence: event.sequence, data: event })),
    ...model.toolCalls.map((trace, index) => ({ kind: "toolCall" as const, tick: trace.tick, sequence: -index, data: trace })),
    ...model.humanTurns.map((turn, index) => ({
      kind: "humanTurn" as const,
      tick: turn.tick,
      sequence: -index,
      data: turn
    })),
    ...model.actionResults.map((result, index) => ({
      kind: "actionResult" as const,
      tick: result.tick,
      sequence: -index,
      data: result
    }))
  ];
  // Most recent first: higher tick first, and within a tick prefer higher
  // sequence (events carry a real monotonic sequence; tool calls/action
  // results are already newest-first arrays so we preserve their order via
  // a descending synthetic sequence).
  return items.sort((a, b) => (b.tick - a.tick) || (b.sequence - a.sequence));
}

export function SimulationActivityFeed({ model, dispatch }: Props) {
  const { locale, t } = useI18n();
  const { syncEvents } = useSimulator(model, dispatch);
  const [showExportMenu, setShowExportMenu] = useState(false);
  const [page, setPage] = useState(0);

  const feed = useMemo(
    () => buildFeed(model),
    [model.events, model.toolCalls, model.humanTurns, model.actionResults]
  );
  const pendingCount = model.toolCalls.filter((trace) => pendingRequestId(trace, model.actionResults)).length;

  const totalPages = Math.max(1, Math.ceil(feed.length / APP_CONFIG.EVENTS_PER_PAGE));
  const startIndex = page * APP_CONFIG.EVENTS_PER_PAGE;
  const displayed = feed.slice(startIndex, startIndex + APP_CONFIG.EVENTS_PER_PAGE);

  async function resolve(requestId: string, decision: "approve" | "reject") {
    try {
      if (decision === "approve") await simulatorClient.approveAction(requestId);
      else await simulatorClient.rejectAction(requestId, t("operatorRejectedReason"));
      await syncEvents();
    } catch (error) {
      dispatch({
        type: "commandRejected",
        error: {
          code: "SIMULATOR_COMMAND_FAILED",
          message: describeError(error, t("approvalCommandFailed")),
          correlationId: "desktop-approval",
          runId: model.runId,
          tick: model.tick
        }
      });
    }
  }

  async function cancelPending() {
    try {
      await simulatorClient.cancelAgentTurn();
      await syncEvents();
    } catch (error) {
      dispatch({
        type: "commandRejected",
        error: {
          code: "SIMULATOR_COMMAND_FAILED",
          message: describeError(error, t("cancelCommandFailed")),
          correlationId: "desktop-cancel",
          runId: model.runId,
          tick: model.tick
        }
      });
    }
  }

  return (
    <section className="flex h-full min-w-0 flex-col overflow-hidden border border-zinc-800 bg-zinc-900/70">
      <div className="flex shrink-0 items-center justify-between border-b border-zinc-800 px-3 py-2 text-sm font-medium">
        <div className="flex items-center gap-2">
          <span>{t("activity")}</span>
          {pendingCount > 0 && (
            <span className="flex items-center gap-1 rounded-full bg-amber-500/90 px-2 py-0.5 text-[10px] font-semibold text-zinc-950">
              {pendingCount} {t("awaitingApproval")}
            </span>
          )}
        </div>
        <div className="flex items-center gap-2">
          {totalPages > 1 && (
            <div className="flex items-center gap-1 text-xs text-zinc-400">
              <button
                aria-label={t("previousPage")}
                className="control-button h-6 w-6 disabled:opacity-30"
                disabled={page === 0}
                onClick={() => setPage(page - 1)}
              >
                <ChevronLeft className="h-3 w-3" />
              </button>
              <span>
                {page + 1} / {totalPages}
              </span>
              <button
                aria-label={t("nextPage")}
                className="control-button h-6 w-6 disabled:opacity-30"
                disabled={page >= totalPages - 1}
                onClick={() => setPage(page + 1)}
              >
                <ChevronRight className="h-3 w-3" />
              </button>
            </div>
          )}
          {feed.length > 0 && (
            <div className="relative">
              <button
                aria-label={t("exportActivity")}
                className="control-button h-6 w-6"
                onClick={() => setShowExportMenu(!showExportMenu)}
              >
                <Download className="h-3 w-3" />
              </button>
              {showExportMenu && (
                <div className="absolute right-0 top-8 z-10 flex flex-col border border-zinc-700 bg-zinc-900 text-xs">
                  <button
                    className="px-3 py-2 text-left hover:bg-zinc-800"
                    onClick={() => {
                      exportEventsAsJSON(model.events);
                      setShowExportMenu(false);
                    }}
                  >
                    {t("exportEventsJson")}
                  </button>
                  <button
                    className="px-3 py-2 text-left hover:bg-zinc-800"
                    onClick={() => {
                      exportEventsAsCSV(model.events);
                      setShowExportMenu(false);
                    }}
                  >
                    {t("exportEventsCsv")}
                  </button>
                  <button
                    className="px-3 py-2 text-left hover:bg-zinc-800"
                    onClick={() => {
                      exportTracesAsJSON(model.toolCalls);
                      setShowExportMenu(false);
                    }}
                  >
                    {t("exportToolsJson")}
                  </button>
                  <button
                    className="px-3 py-2 text-left hover:bg-zinc-800"
                    onClick={() => {
                      exportTracesAsCSV(model.toolCalls);
                      setShowExportMenu(false);
                    }}
                  >
                    {t("exportToolsCsv")}
                  </button>
                  <button
                    className="px-3 py-2 text-left hover:bg-zinc-800"
                    onClick={() => {
                      exportActionResultsAsJSON(model.actionResults);
                      setShowExportMenu(false);
                    }}
                  >
                    {t("exportActionsJson")}
                  </button>
                </div>
              )}
            </div>
          )}
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        {feed.length === 0 ? (
          <div className="p-3 text-sm text-zinc-500">
            {t("emptyActivity")}
          </div>
        ) : (
          displayed.map((item) => {
            if (item.kind === "event") {
              const event = item.data;
              return (
                <div
                  key={`event-${event.eventId}`}
                  className="grid grid-cols-[52px_24px_1fr] items-start gap-2 border-b border-zinc-800/60 px-3 py-1.5 text-xs"
                >
                  <span className="text-zinc-500">t{event.tick}</span>
                  <span className="text-zinc-500" title={t("worldEvent")}>
                    <Zap className="h-3.5 w-3.5" />
                  </span>
                  <div className="min-w-0">
                    <div>
                      <span className="text-cyan-200">{eventLabel(event.eventType, locale)}</span>
                      <code className="ml-2 text-[10px] text-zinc-600">{event.eventType}</code>
                      <div className="mt-0.5 text-zinc-400">
                        {eventDescription(event.eventType, event.payload.message, locale)}
                      </div>
                    </div>
                    <div className="mt-1 flex flex-wrap gap-x-3 gap-y-0.5 font-mono text-[10px] text-zinc-600">
                      <span>{t("eventSource")}: {event.source}</span>
                      {event.payload.target ? <span>{t("target")}: {event.payload.target}</span> : null}
                      {typeof event.payload.value === "number" && Number.isFinite(event.payload.value) ? (
                        <span>{t("value")}: {event.payload.value.toFixed(3)}</span>
                      ) : null}
                      <span>{t("priority")}: {event.priority}</span>
                      <span className="max-w-64 truncate" title={event.correlationId}>
                        {t("correlation")}: {event.correlationId}
                      </span>
                    </div>
                  </div>
                </div>
              );
            }

            if (item.kind === "toolCall") {
              const trace = item.data;
              const requestId = pendingRequestId(trace, model.actionResults);
              return (
                <div
                  key={`tool-${trace.callId}`}
                  className={`grid grid-cols-[52px_24px_1fr] items-start gap-2 border-b border-zinc-800/60 px-3 py-1.5 text-xs ${
                    requestId ? "bg-amber-950/30" : ""
                  }`}
                >
                  <span className="text-zinc-500">t{trace.tick}</span>
                  <span className="text-sky-400" title={t("toolCall")}>
                    <Wrench className="h-3.5 w-3.5" />
                  </span>
                  <div>
                    <div>
                      <span className="text-sky-300">{trace.toolName}</span>
                      <span className="ml-2 text-zinc-400">
                        {trace.allowed ? t("allowed") : t("denied")}
                        {trace.sideEffect ? ` / ${t("mutation")}` : ` / ${t("readOnly")}`}
                      </span>
                    </div>
                    {requestId ? (
                      <div className="mt-1.5 flex items-center gap-2">
                        <span className="text-[11px] font-medium text-amber-300">{t("pendingApproval")}</span>
                        <button
                          aria-label={t("approveAction")}
                          className="control-button h-6 w-6"
                          title={t("approveAction")}
                          onClick={() => void resolve(requestId, "approve")}
                        >
                          <Check className="h-3 w-3" />
                        </button>
                        <button
                          aria-label={t("rejectAction")}
                          className="control-button h-6 w-6"
                          title={t("rejectAction")}
                          onClick={() => void resolve(requestId, "reject")}
                        >
                          <X className="h-3 w-3" />
                        </button>
                        <button
                          aria-label={t("cancelPending")}
                          className="control-button h-6 w-6"
                          title={t("cancelPending")}
                          onClick={() => void cancelPending()}
                        >
                          <Ban className="h-3 w-3" />
                        </button>
                      </div>
                    ) : null}
                  </div>
                </div>
              );
            }

            if (item.kind === "humanTurn") {
              const turn = item.data;
              const delta = turn.evidence.decision.internalStateDelta;
              const deltaEntries = Object.entries(delta).filter(([, value]) => value !== undefined);
              return (
                <div
                  key={`human-${turn.tick}-${turn.evidence.humanId}`}
                  className="grid grid-cols-[52px_24px_1fr] items-start gap-2 border-b border-zinc-800/60 bg-violet-950/10 px-3 py-2 text-xs"
                >
                  <span className="text-zinc-500">t{turn.tick}</span>
                  <span className="text-violet-300" title={t("humanTurn")}>
                    <Bot className="h-3.5 w-3.5" />
                  </span>
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-x-2">
                      <span className="font-medium text-violet-200">{turn.evidence.humanId}</span>
                      <code className="text-[10px] text-zinc-500">{turn.backend}</code>
                    </div>
                    <div className="mt-1 grid gap-1 text-[10px] text-zinc-500 sm:grid-cols-2">
                      <div>
                        <span className="text-zinc-400">{t("requestedActions")}: </span>
                        {turn.evidence.decision.actions.length > 0
                          ? turn.evidence.decision.actions
                              .map((action) => `${commandLabel(action.command, locale)} (${action.command}) → ${action.target}`)
                              .join(", ")
                          : t("noRequestedActions")}
                      </div>
                      <div>
                        <span className="text-zinc-400">{t("internalDelta")}: </span>
                        {deltaEntries.length > 0
                          ? deltaEntries
                              .map(([name, value]) => `${name} ${Number(value) >= 0 ? "+" : ""}${Number(value).toFixed(3)}`)
                              .join(", ")
                          : t("noInternalDelta")}
                      </div>
                    </div>
                  </div>
                </div>
              );
            }

            const result = item.data;
            return (
              <div
                key={`action-${result.request.requestId}`}
                className="grid grid-cols-[52px_24px_1fr] items-start gap-2 border-b border-zinc-800/60 px-3 py-1.5 text-xs"
              >
                <span className="text-zinc-500">t{result.tick}</span>
                <span
                  className={result.status === "applied" ? "text-emerald-400" : "text-amber-400"}
                  title={t("actionResult")}
                >
                  <Zap className="h-3.5 w-3.5" />
                </span>
                <span>
                  <span className="text-emerald-300">{commandLabel(result.request.command, locale)}</span>
                  <code className="ml-2 text-[10px] text-zinc-600">{result.request.command}</code>
                  <span className="ml-2 text-zinc-400">
                    {t("onTarget")} {result.request.target} · {actionStatusLabel(result.status, locale)}
                    {result.errorCode ? ` / ${result.errorCode}` : ""}
                  </span>
                </span>
              </div>
            );
          })
        )}
      </div>
    </section>
  );
}
