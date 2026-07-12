import { useState } from "react";
import { ChevronLeft, ChevronRight, Download } from "lucide-react";
import { APP_CONFIG } from "../config/constants";
import { exportEventsAsCSV, exportEventsAsJSON } from "../utils/export";
import type { SimulationModel } from "../types/simulation";

export function SimulationTimeline({ model }: { model: SimulationModel }) {
  const [page, setPage] = useState(0);
  const [showExportMenu, setShowExportMenu] = useState(false);

  const totalPages = Math.ceil(model.events.length / APP_CONFIG.EVENTS_PER_PAGE);
  const startIndex = page * APP_CONFIG.EVENTS_PER_PAGE;
  const endIndex = startIndex + APP_CONFIG.EVENTS_PER_PAGE;
  const displayedEvents = model.events.slice(startIndex, endIndex);

  return (
    <section className="min-h-[260px] border border-zinc-800 bg-zinc-900/70">
      <div className="flex items-center justify-between border-b border-zinc-800 px-3 py-2 text-sm font-medium">
        <span>Timeline</span>
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
          {model.events.length > 0 && (
            <div className="relative">
              <button
                aria-label="Export events"
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
                    Export as JSON
                  </button>
                  <button
                    className="px-3 py-2 text-left hover:bg-zinc-800"
                    onClick={() => {
                      exportEventsAsCSV(model.events);
                      setShowExportMenu(false);
                    }}
                  >
                    Export as CSV
                  </button>
                </div>
              )}
            </div>
          )}
        </div>
      </div>
      <div className="max-h-[340px] overflow-auto">
        {displayedEvents.length === 0 ? (
          <div className="p-3 text-sm text-zinc-500">
            {model.events.length === 0 ? "No events" : "No events on this page"}
          </div>
        ) : (
          displayedEvents.map((event) => (
            <div key={event.eventId} className="grid grid-cols-[70px_160px_1fr] gap-3 border-b border-zinc-800 px-3 py-2 text-sm">
              <span className="text-zinc-400">t{event.tick}</span>
              <span className="text-cyan-200">{event.eventType}</span>
              <span className="text-zinc-300">{event.payload.message}</span>
            </div>
          ))
        )}
      </div>
    </section>
  );
}
