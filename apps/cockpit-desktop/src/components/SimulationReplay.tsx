import { useState } from "react";
import { GitCompareArrows, Play, FolderOpen } from "lucide-react";
import { useRunner } from "../hooks/useRunner";
import { runnerClient } from "../runnerClient";
import type { SimulationAction } from "../state/simulationReducer";
import type { RecordingDiff, SimulationModel } from "../types/simulation";

interface Props {
  model: SimulationModel;
  dispatch: React.Dispatch<SimulationAction>;
}

function reportFailure(dispatch: React.Dispatch<SimulationAction>, model: SimulationModel, error: unknown) {
  dispatch({
    type: "commandRejected",
    error: {
      code: "RUNNER_COMMAND_FAILED",
      message: error instanceof Error ? error.message : "replay command failed",
      correlationId: "desktop-replay",
      runId: model.runId,
      tick: model.tick
    }
  });
}

function DiffSummary({ report }: { report: RecordingDiff }) {
  const divergence = report.firstDivergence;
  return (
    <div className="space-y-1 border-t border-zinc-800 pt-2 text-xs text-zinc-300">
      <div className={report.equivalent ? "text-emerald-300" : "text-amber-300"}>
        {report.equivalent ? "Equivalent recordings" : "Recording divergence"}
      </div>
      {divergence ? <div>First divergent tick: {divergence.tick}</div> : null}
      <div>
        source {report.sourceMetrics.ticks} ticks / candidate {report.candidateMetrics.ticks} ticks
      </div>
    </div>
  );
}

export function SimulationReplay({ model, dispatch }: Props) {
  const { syncEvents } = useRunner(model, dispatch);
  const [recordingPath, setRecordingPath] = useState("");
  const [candidatePath, setCandidatePath] = useState("");

  async function replay() {
    if (!model.scenario || !recordingPath) return;
    try {
      await runnerClient.startReplay(model.scenario.path, recordingPath);
      await syncEvents();
    } catch (error) {
      reportFailure(dispatch, model, error);
    }
  }

  async function compare() {
    if (!recordingPath || !candidatePath) return;
    try {
      const report = await runnerClient.diffRecordings(recordingPath, candidatePath);
      dispatch({ type: "replayDiffUpdated", report });
    } catch (error) {
      reportFailure(dispatch, model, error);
    }
  }

  async function browseRecording(target: "source" | "candidate") {
    const path = await runnerClient.openRecordingFilePicker();
    if (path) {
      if (target === "source") setRecordingPath(path);
      else setCandidatePath(path);
    }
  }

  return (
    <section className="border border-zinc-800 bg-zinc-900/70">
      <div className="border-b border-zinc-800 px-3 py-2 text-sm font-medium">Replay</div>
      <div className="space-y-2 p-3">
        <div className="flex gap-2">
          <input
            aria-label="Recording path"
            className="h-8 flex-1 border border-zinc-700 bg-zinc-950 px-2 text-xs text-zinc-100"
            placeholder="Recording path"
            value={recordingPath}
            onChange={(event) => setRecordingPath(event.target.value)}
          />
          <button
            aria-label="Browse recording"
            className="control-button h-8 w-8"
            onClick={() => void browseRecording("source")}
          >
            <FolderOpen className="h-3 w-3" />
          </button>
        </div>
        <div className="grid grid-cols-2 gap-2">
          <button aria-label="Replay recording" className="control-button" disabled={!model.scenario || !recordingPath} onClick={() => void replay()}>
            <Play className="h-4 w-4" />
          </button>
          <button aria-label="Compare recordings" className="control-button" disabled={!recordingPath || !candidatePath} onClick={() => void compare()}>
            <GitCompareArrows className="h-4 w-4" />
          </button>
        </div>
        <div className="flex gap-2">
          <input
            aria-label="Comparison recording path"
            className="h-8 flex-1 border border-zinc-700 bg-zinc-950 px-2 text-xs text-zinc-100"
            placeholder="Comparison recording path"
            value={candidatePath}
            onChange={(event) => setCandidatePath(event.target.value)}
          />
          <button
            aria-label="Browse comparison recording"
            className="control-button h-8 w-8"
            onClick={() => void browseRecording("candidate")}
          >
            <FolderOpen className="h-3 w-3" />
          </button>
        </div>
        {model.replayDiff ? <DiffSummary report={model.replayDiff} /> : null}
      </div>
    </section>
  );
}
