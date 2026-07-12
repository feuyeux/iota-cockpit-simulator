import { useState } from "react";
import { Ban, Check, X, Download, ChevronLeft, ChevronRight } from "lucide-react";
import { APP_CONFIG } from "../config/constants";
import { useRunner } from "../hooks/useRunner";
import { exportTracesAsCSV, exportTracesAsJSON, exportActionResultsAsJSON } from "../utils/export";
import { runnerClient } from "../runnerClient";
import type { SimulationAction } from "../state/simulationReducer";
import type { SimulationModel } from "../types/simulation";

interface Props {
  model: SimulationModel;
  dispatch: React.Dispatch<SimulationAction>;
}

function pendingRequestId(
  trace: SimulationModel["toolCalls"][number],
  actionResults: SimulationModel["actionResults"]
): string | undefined {
  if (trace.toolName !== "simulation.request_action" || !isPendingApproval(trace.result)) return undefined;
  return actionResults.some((result) => result.request.requestId === trace.callId) ? undefined : trace.callId;
}

function isPendingApproval(result: unknown): boolean {
  if (!isRecord(result) || !isRecord(result.result)) return false;
  return result.result.status === "pendingApproval";
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function SimulationTrace({ model, dispatch }: Props) {
  const { syncEvents } = useRunner(model, dispatch);
  const [showExportMenu, setShowExportMenu] = useState(false);
  const [page, setPage] = useState(0);

  const totalItems = model.toolCalls.length + model.actionResults.length;
  const totalPages = Math.ceil(totalItems / APP_CONFIG.TRACES_PER_PAGE);

  async function resolve(requestId: string, decision: "approve" | "reject") {
    try {
      if (decision === "approve") await runnerClient.approveAction(requestId);
      else await runnerClient.rejectAction(requestId, "operator rejected action");
      await syncEvents();
    } catch (error) {
      dispatch({
        type: "commandRejected",
        error: {
          code: "RUNNER_COMMAND_FAILED",
          message: error instanceof Error ? error.message : "approval command failed",
          correlationId: "desktop-approval",
          runId: model.runId,
          tick: model.tick
        }
      });
    }
  }

  async function cancelPending() {
    try {
      await runnerClient.cancelAgentTurn();
      await syncEvents();
    } catch (error) {
      dispatch({
        type: "commandRejected",
        error: {
          code: "RUNNER_COMMAND_FAILED",
          message: error instanceof Error ? error.message : "cancel command failed",
          correlationId: "desktop-cancel",
          runId: model.runId,
          tick: model.tick
        }
      });
    }
  }

  const startIndex = page * APP_CONFIG.TRACES_PER_PAGE;
  const endIndex = startIndex + APP_CONFIG.TRACES_PER_PAGE;
  const displayedToolCalls = model.toolCalls.slice(startIndex, Math.min(endIndex, model.toolCalls.length));
  const remainingSlots = endIndex - model.toolCalls.length;
  const displayedActionResults = remainingSlots > 0
    ? model.actionResults.slice(0, remainingSlots)
    : [];

  return (
    <section className="min-h-[260px] border border-zinc-800 bg-zinc-900/70">
      <div className="flex items-center justify-between border-b border-zinc-800 px-3 py-2 text-sm font-medium">
        <span>Agent Trace</span>
        <div className="flex items-center gap-2">
          {totalPages > 1 && (
            <div className="flex items-center gap-1 text-xs text-zinc-400">
              <button
                aria-label="Previous page"
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
                aria-label="Next page"
                className="control-button h-6 w-6 disabled:opacity-30"
                disabled={page >= totalPages - 1}
                onClick={() => setPage(page + 1)}
              >
                <ChevronRight className="h-3 w-3" />
              </button>
            </div>
          )}
          {totalItems > 0 && (
            <div className="relative">
              <button
                aria-label="Export traces"
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
                      exportTracesAsJSON(model.toolCalls);
                      setShowExportMenu(false);
                    }}
                  >
                    Export traces as JSON
                  </button>
                  <button
                    className="px-3 py-2 text-left hover:bg-zinc-800"
                    onClick={() => {
                      exportTracesAsCSV(model.toolCalls);
                      setShowExportMenu(false);
                    }}
                  >
                    Export traces as CSV
                  </button>
                  <button
                    className="px-3 py-2 text-left hover:bg-zinc-800"
                    onClick={() => {
                      exportActionResultsAsJSON(model.actionResults);
                      setShowExportMenu(false);
                    }}
                  >
                    Export actions as JSON
                  </button>
                </div>
              )}
            </div>
          )}
        </div>
      </div>
      <div className="max-h-[340px] overflow-auto">
        {model.actionResults.length === 0 && model.toolCalls.length === 0 ? (
          <div className="p-3 text-sm text-zinc-500">No tool calls</div>
        ) : (
          <>
            {displayedToolCalls.map((trace) => {
              const requestId = pendingRequestId(trace, model.actionResults);
              return (
              <div key={trace.callId} className="border-b border-zinc-800 px-3 py-2 text-sm">
                <div className="flex justify-between">
                  <span className="text-cyan-200">{trace.toolName}</span>
                  <span className="text-zinc-400">t{trace.tick}</span>
                </div>
                <div className="mt-1 text-zinc-300">
                  {trace.allowed ? "allowed" : "denied"}
                  {trace.sideEffect ? " / mutation" : " / read-only"}
                </div>
                {requestId ? (
                  <div className="mt-2 flex items-center gap-2">
                    <span className="text-xs text-amber-300">pending approval</span>
                    <button aria-label="Approve action" className="control-button h-7 w-7" title="Approve action" onClick={() => void resolve(requestId, "approve")}>
                      <Check className="h-3 w-3" />
                    </button>
                    <button aria-label="Reject action" className="control-button h-7 w-7" title="Reject action" onClick={() => void resolve(requestId, "reject")}>
                      <X className="h-3 w-3" />
                    </button>
                    <button aria-label="Cancel pending actions" className="control-button h-7 w-7" title="Cancel pending actions" onClick={() => void cancelPending()}>
                      <Ban className="h-3 w-3" />
                    </button>
                  </div>
                ) : null}
              </div>
              );
            })}
            {displayedActionResults.map((result) => (
              <div key={result.request.requestId} className="border-b border-zinc-800 px-3 py-2 text-sm">
                <div className="flex justify-between">
                  <span className="text-cyan-200">{result.request.command}</span>
                  <span className="text-zinc-400">t{result.tick}</span>
                </div>
                <div className="mt-1 text-zinc-300">
                  {result.status}
                  {result.errorCode ? ` / ${result.errorCode}` : ""}
                </div>
              </div>
            ))}
          </>
        )}
      </div>
    </section>
  );
}
