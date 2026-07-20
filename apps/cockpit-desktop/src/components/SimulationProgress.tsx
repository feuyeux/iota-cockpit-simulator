import { useI18n } from "../i18n";
import type { SimulationModel } from "../types/simulation";

interface Props {
  tick: number;
  deadlineTick?: number;
  state: SimulationModel["state"];
}

export function SimulationProgress({ tick, deadlineTick, state }: Props) {
  const { t } = useI18n();
  const hasDeadline = Number.isFinite(deadlineTick) && (deadlineTick ?? 0) > 0;

  if (!hasDeadline) {
    return (
      <div className="flex min-w-0 items-center gap-2 text-xs text-zinc-500" data-testid="simulation-progress-pending">
        <span className="font-medium text-zinc-400">{t("simulationProgress")}:</span>
        <span>{t("progressPending")}</span>
      </div>
    );
  }

  const deadline = deadlineTick!;
  const completedTicks = Math.min(Math.max(tick, 0), deadline);
  const remainingTicks = Math.max(deadline - completedTicks, 0);
  const percent = Math.round((completedTicks / deadline) * 100);
  const deadlineReached = remainingTicks === 0 || state === "completed";
  const status = deadlineReached
    ? t("deadlineReached")
    : `${t("remainingSteps")} ${remainingTicks} ${t("ticksUnit")}`;

  return (
    <section className="flex w-full min-w-0 items-center gap-3 text-xs" aria-label={t("simulationProgress")} data-testid="simulation-progress">
      <span className="shrink-0 font-medium text-zinc-400">{t("simulationProgress")}</span>
      <div className="flex flex-1 items-center min-w-[80px]">
        <div
          aria-label={t("simulationProgress")}
          aria-valuemax={deadline}
          aria-valuemin={0}
          aria-valuenow={completedTicks}
          aria-valuetext={`${percent}% · ${status}`}
          className="h-2 w-full overflow-hidden rounded-full bg-zinc-900 border border-zinc-800"
          role="progressbar"
        >
          <div
            className={deadlineReached ? "h-full bg-emerald-400 transition-[width] duration-300" : "h-full bg-cyan-400 transition-[width] duration-300"}
            style={{ width: `${percent}%` }}
          />
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-1.5 font-mono text-zinc-300">
        <span className="text-cyan-300 font-semibold">t{completedTicks} / t{deadline}</span>
        <span className={deadlineReached ? "text-emerald-400 font-medium" : "text-zinc-400"}>
          ({status})
        </span>
      </div>
    </section>
  );
}
