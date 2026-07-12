import { useCallback } from "react";
import { runnerClient } from "../runnerClient";
import type { SimulationAction } from "../state/simulationReducer";
import type { SimulationModel } from "../types/simulation";

export function useRunner(model: SimulationModel, dispatch: React.Dispatch<SimulationAction>) {
  const syncEvents = useCallback(async () => {
    const batch = await runnerClient.snapshot(model.lastCursor);
    if (batch.resetRequired) {
      const snapshot = await runnerClient.simulationSnapshot();
      dispatch({ type: "snapshotReset", snapshot, cursor: batch.firstAvailableCursor - 1 });
    }
    for (const event of batch.events) dispatch({ type: "runnerEvent", event });
  }, [model.lastCursor, dispatch]);

  const runCommand = useCallback(
    async (command: () => Promise<void>): Promise<boolean> => {
      try {
        await command();
        await syncEvents();
        return true;
      } catch (error) {
        dispatch({
          type: "commandRejected",
          error: {
            code: "RUNNER_COMMAND_FAILED",
            message: error instanceof Error ? error.message : "command failed",
            runId: model.runId,
            tick: model.tick,
            correlationId: "desktop-command",
          },
        });
        return false;
      }
    },
    [syncEvents, dispatch, model.runId, model.tick]
  );

  return { syncEvents, runCommand };
}
