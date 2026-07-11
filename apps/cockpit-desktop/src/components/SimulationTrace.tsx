import { Ban, Check, X } from "lucide-react";
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
  async function syncEvents() {
    try {
      const batch = await runnerClient.snapshot(model.lastCursor);
      if (batch.resetRequired) {
        const snapshot = await runnerClient.simulationSnapshot();
        dispatch({ type: "snapshotReset", snapshot, cursor: batch.firstAvailableCursor - 1 });
      }
      for (const event of batch.events) dispatch({ type: "runnerEvent", event });
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

  return (
    <section className="min-h-[260px] border border-zinc-800 bg-zinc-900/70">
      <div className="border-b border-zinc-800 px-3 py-2 text-sm font-medium">Agent Trace</div>
      <div className="max-h-[340px] overflow-auto">
        {model.actionResults.length === 0 && model.toolCalls.length === 0 ? (
          <div className="p-3 text-sm text-zinc-500">No tool calls</div>
        ) : (
          <>
            {model.toolCalls.map((trace) => {
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
            {model.actionResults.map((result) => (
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
