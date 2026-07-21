import { MessageSquare, Ear, User } from "lucide-react";
import type { SimulationModel, PerceivedEvent } from "../types/simulation";
import { useI18n, type MessageKey } from "../i18n";

/// Chat-like feed of what each person has perceived and said, derived from the
/// world snapshot's per-human short-term memory. Utterances (kind === "utterance")
/// are rendered as speech; other perceived events are rendered as sensed input.
/// This surfaces the social/perception dynamics of the world model over time
/// without needing a separate IPC channel: the snapshot already carries each
/// human's delivered perception queue.
function eventRow(humanName: string, event: PerceivedEvent, index: number, t: (key: MessageKey) => string) {
  const isUtterance = event.kind === "utterance";
  return (
    <div
      key={`${event.originTick}-${event.source}-${index}`}
      className="flex items-start gap-2 border-b border-zinc-800/60 px-2 py-1.5 text-xs"
    >
      <span className="mt-0.5 shrink-0 text-zinc-500">
        {isUtterance ? (
          <MessageSquare className="h-3.5 w-3.5 text-sky-300" />
        ) : (
          <Ear className="h-3.5 w-3.5 text-zinc-400" />
        )}
      </span>
      <div className="min-w-0">
        <div className="text-zinc-500">
          <span className="text-zinc-400">t{event.availableAtTick}</span>
          {" · "}
          {isUtterance ? (
            <span className="text-sky-300">{event.source} {t("said")}</span>
          ) : (
            <span>
              {humanName} {t("sensed")} {event.kind}
            </span>
          )}
        </div>
        <div className={isUtterance ? "text-sky-100" : "text-zinc-300"}>{event.summary}</div>
      </div>
    </div>
  );
}

export function SimulationNarrative({ model }: { model: SimulationModel }) {
  const { t } = useI18n();
  const humans = model.snapshot?.humans ?? [];

  return (
    <section className="flex h-full min-h-0 flex-1 flex-col overflow-hidden bg-zinc-900/60 backdrop-blur-sm">
      <div className="flex shrink-0 items-center justify-between border-b border-zinc-800/80 bg-zinc-900/80 px-3.5 py-2 text-xs font-semibold text-zinc-100">
        <span className="tracking-wide">{t("dialoguePerception")}</span>
        <span className="text-[11px] font-normal text-zinc-400">{t("perPersonFeed")}</span>
      </div>
      {humans.length === 0 ? (
        <div className="p-4 text-xs text-zinc-500">
          {t("noHumans")}
        </div>
      ) : (
        <div className="grid min-h-0 flex-1 gap-3 overflow-y-auto p-3 md:grid-cols-2">
          {humans.map((human) => {
            // Most recent first, capped so the feed stays readable.
            const recent = [...human.shortTermMemory]
              .sort((a, b) => b.availableAtTick - a.availableAtTick)
              .slice(0, 12);
            return (
              <div
                key={human.id}
                className="flex min-h-0 flex-col overflow-hidden rounded border border-zinc-800 bg-zinc-950/50"
              >
                <div className="flex shrink-0 items-center gap-1.5 border-b border-zinc-800 px-2 py-1.5 text-xs">
                  <User className="h-3.5 w-3.5 text-emerald-300" />
                  <span className="font-medium">{human.persona.name}</span>
                  <span className="text-zinc-500">
                    {human.persona.role} · {human.location}
                  </span>
                </div>
                <div className="min-h-0 flex-1 overflow-y-auto">
                  {recent.length === 0 ? (
                    <div className="px-2 py-2 text-[11px] text-zinc-600">
                      ({t("nothingPerceived")})
                    </div>
                  ) : (
                    recent.map((event, index) => eventRow(human.persona.name, event, index, t))
                  )}
                </div>
                {human.longTermMemory.length > 0 && (
                  <div className="border-t border-zinc-800 px-2 py-1.5 text-[10px] text-zinc-500">
                    {human.longTermMemory.length} {t("longTermMemories")}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}
