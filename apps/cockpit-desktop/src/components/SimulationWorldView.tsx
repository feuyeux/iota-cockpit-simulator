import { RadioTower, Wind, AlertCircle } from "lucide-react";
import type { SimulationModel } from "../types/simulation";

export function SimulationWorldView({ model }: { model: SimulationModel }) {
  const snapshot = model.snapshot;
  const observations = model.observations;
  const latestObservation = observations[0];
  const sensorDegraded = latestObservation?.quality.degraded ?? false;

  return (
    <section className="min-h-[360px] border border-zinc-800 bg-zinc-900/70">
      <div className="flex items-center justify-between border-b border-zinc-800 px-3 py-2 text-sm font-medium">
        <span>World</span>
        <div className="flex items-center gap-2">
          {sensorDegraded && (
            <span className="flex items-center gap-1 text-xs text-amber-300">
              <AlertCircle className="h-3 w-3" />
              Sensor degraded
            </span>
          )}
          <span className="text-xs text-zinc-400">Ground Truth hidden</span>
        </div>
      </div>
      <div className="grid h-[calc(100%-37px)] grid-cols-[180px_1fr]">
        <aside className="border-r border-zinc-800 p-3 text-sm text-zinc-300">
          {["cabin", "pilot-1", "engine-1", "alarm-1"].map((entity) => (
            <div key={entity} className="mb-2 flex items-center gap-2">
              <RadioTower className="h-4 w-4 text-cyan-300" />
              {entity}
            </div>
          ))}
          {latestObservation && (
            <div className="mt-4 space-y-1 border-t border-zinc-800 pt-2 text-xs">
              <div className="text-zinc-400">Sensor Quality</div>
              <div>Visibility: {(latestObservation.quality.visibilityQuality * 100).toFixed(0)}%</div>
              <div>Audio: {(latestObservation.quality.audioQuality * 100).toFixed(0)}%</div>
              <div>Confidence: {(latestObservation.quality.confidence * 100).toFixed(0)}%</div>
            </div>
          )}
        </aside>
        <div className="relative overflow-hidden p-4">
          <div className="absolute inset-6 border border-zinc-700 bg-zinc-950">
            <div
              className="absolute inset-0 bg-zinc-300/10"
              style={{ opacity: snapshot ? 1 - snapshot.environment.visibility : 0.1 }}
            />
            {sensorDegraded && (
              <div className="absolute inset-0 border-2 border-amber-400/30" />
            )}
            <div className="absolute left-8 top-8 h-16 w-32 border border-cyan-400/70 bg-cyan-950/50" />
            <div className="absolute bottom-10 right-10 h-20 w-24 border border-amber-400/70 bg-amber-950/40" />
            <div className="absolute left-1/2 top-1/2 flex -translate-x-1/2 -translate-y-1/2 items-center gap-2 text-sm text-zinc-200">
              <Wind className="h-4 w-4 text-zinc-300" />
              visibility {snapshot ? snapshot.environment.visibility.toFixed(2) : "-"}
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
