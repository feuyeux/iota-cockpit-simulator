import { useCallback } from "react";
import { simulatorClient } from "../simulatorClient";
import type { SimulationAction } from "../state/simulationReducer";
import type { SimulationModel } from "../types/simulation";
import { useI18n } from "../i18n";
import { describeError } from "../utils/describeError";

export function useSimulator(model: SimulationModel, dispatch: React.Dispatch<SimulationAction>) {
  const { t } = useI18n();
  const syncEvents = useCallback(async () => {
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
    return batch;
  }, [model.lastCursor, dispatch]);

  const runCommand = useCallback(
    async (command: () => Promise<unknown>): Promise<boolean> => {
      try {
        await command();
        await syncEvents();
        return true;
      } catch (error) {
        try {
          const batch = await syncEvents();
          if (batch.events.some((event) => event.type === "SimulationError")) return false;
        } catch {
          // The original command failure remains useful when event recovery is unavailable.
        }
        dispatch({
          type: "commandRejected",
          error: {
            code: "SIMULATOR_COMMAND_FAILED",
            message: describeError(error, t("commandFailed")),
            runId: model.runId,
            tick: model.tick,
            correlationId: "desktop-command",
          },
        });
        return false;
      }
    },
    [syncEvents, dispatch, model.runId, model.tick, t]
  );

  return { syncEvents, runCommand };
}
