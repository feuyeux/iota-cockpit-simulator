import type { SimulationEvent, ToolCallTrace, ActionResult } from "../types/simulation";
import { redactValue } from "./redact";

export function exportEventsAsJSON(events: SimulationEvent[]): void {
  const data = JSON.stringify(redactValue(events), null, 2);
  downloadFile(data, `cockpit-events-${Date.now()}.json`, "application/json");
}

export function exportEventsAsCSV(events: SimulationEvent[]): void {
  const headers = ["eventId", "eventType", "runId", "tick", "source", "priority", "sequence", "correlationId", "message", "target", "value"];
  const rows = events.map((event) => [
    event.eventId,
    event.eventType,
    event.runId,
    event.tick,
    event.source,
    event.priority,
    event.sequence,
    event.correlationId,
    event.payload.message,
    event.payload.target ?? "",
    event.payload.value ?? "",
  ]);
  const csv = [headers, ...rows].map((row) => row.map((cell) => escapeCSV(String(cell))).join(",")).join("\n");
  downloadFile(csv, `cockpit-events-${Date.now()}.csv`, "text/csv");
}

export function exportTracesAsJSON(traces: ToolCallTrace[]): void {
  const data = JSON.stringify(redactValue(traces), null, 2);
  downloadFile(data, `cockpit-traces-${Date.now()}.json`, "application/json");
}

export function exportTracesAsCSV(traces: ToolCallTrace[]): void {
  const headers = ["callId", "toolName", "runId", "agentId", "tick", "correlationId", "allowed", "sideEffect"];
  const rows = traces.map((trace) => [
    trace.callId,
    trace.toolName,
    trace.runId,
    trace.agentId,
    trace.tick,
    trace.correlationId,
    trace.allowed,
    trace.sideEffect,
  ]);
  const csv = [headers, ...rows].map((row) => row.map((cell) => escapeCSV(String(cell))).join(",")).join("\n");
  downloadFile(csv, `cockpit-traces-${Date.now()}.csv`, "text/csv");
}

export function exportActionResultsAsJSON(results: ActionResult[]): void {
  const data = JSON.stringify(redactValue(results), null, 2);
  downloadFile(data, `cockpit-actions-${Date.now()}.json`, "application/json");
}

function escapeCSV(value: string): string {
  if (value.includes(",") || value.includes('"') || value.includes("\n")) {
    return `"${value.replace(/"/g, '""')}"`;
  }
  return value;
}

function downloadFile(content: string, filename: string, mimeType: string): void {
  const blob = new Blob([content], { type: mimeType });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  document.body.appendChild(link);
  link.click();
  document.body.removeChild(link);
  URL.revokeObjectURL(url);
}
