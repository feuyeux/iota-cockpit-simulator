import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import type { SimulationEvent, ToolCallTrace } from "../types/simulation";

// Mock the download functionality
const mockCreateObjectURL = vi.fn(() => "blob:mock-url");
const mockRevokeObjectURL = vi.fn();
const mockClick = vi.fn();
const mockAppendChild = vi.fn();
const mockRemoveChild = vi.fn();

beforeEach(() => {
  globalThis.URL.createObjectURL = mockCreateObjectURL;
  globalThis.URL.revokeObjectURL = mockRevokeObjectURL;
  
  vi.spyOn(document, "createElement").mockImplementation((tag) => {
    if (tag === "a") {
      return {
        click: mockClick,
        href: "",
        download: "",
      } as unknown as HTMLAnchorElement;
    }
    return document.createElement(tag);
  });
  
  vi.spyOn(document.body, "appendChild").mockImplementation(mockAppendChild);
  vi.spyOn(document.body, "removeChild").mockImplementation(mockRemoveChild);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("export utilities", () => {
  it("should format CSV correctly with special characters", async () => {
    const { exportEventsAsCSV } = await import("./export");
    
    const events: SimulationEvent[] = [
      {
        eventId: "evt-1",
        eventType: "TestEvent",
        runId: "run-1",
        tick: 5,
        source: "test",
        priority: 1,
        sequence: 0,
        correlationId: "corr-1",
        payload: {
          message: 'Test with "quotes" and, commas',
          target: "target-1",
          value: 42,
        },
      },
    ];

    exportEventsAsCSV(events);

    expect(mockCreateObjectURL).toHaveBeenCalledWith(
      expect.objectContaining({ type: "text/csv" })
    );
    expect(mockClick).toHaveBeenCalled();
    expect(mockRevokeObjectURL).toHaveBeenCalledWith("blob:mock-url");
  });

  it("should export traces as JSON", async () => {
    const { exportTracesAsJSON } = await import("./export");

    const traces: ToolCallTrace[] = [
      {
        callId: "call-1",
        toolName: "test.tool",
        runId: "run-1",
        agentId: "agent-1",
        tick: 3,
        correlationId: "corr-1",
        arguments: { arg1: "value1" },
        result: { success: true },
        sideEffect: false,
        allowed: true,
      },
    ];

    exportTracesAsJSON(traces);

    expect(mockCreateObjectURL).toHaveBeenCalledWith(
      expect.objectContaining({ type: "application/json" })
    );
    expect(mockClick).toHaveBeenCalled();
  });

  it("redacts secrets in the exported trace artifact before download", async () => {
    const { exportTracesAsJSON } = await import("./export");

    const traces: ToolCallTrace[] = [
      {
        callId: "call-1",
        toolName: "test.tool",
        runId: "run-1",
        agentId: "agent-1",
        tick: 3,
        correlationId: "corr-1",
        arguments: {
          apiKey: "api-key-must-not-leak",
          nested: { auth_token: "token-must-not-leak" },
        },
        result: { prompt: "private-prompt-must-not-leak" },
        sideEffect: false,
        allowed: true,
      },
    ];

    exportTracesAsJSON(traces);

    const calls = mockCreateObjectURL.mock.calls;
    expect(calls.length).toBeGreaterThan(0);
    const lastCall = calls[calls.length - 1] as unknown[];
    expect(lastCall.length).toBeGreaterThan(0);
    const blob = lastCall[0] as Blob;
    expect(blob).toBeInstanceOf(Blob);
    
    // Read the blob content via FileReader since jsdom's Blob doesn't have text()
    const text = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(reader.result as string);
      reader.onerror = reject;
      reader.readAsText(blob);
    });
    
    expect(text).not.toContain("api-key-must-not-leak");
    expect(text).not.toContain("token-must-not-leak");
    expect(text).not.toContain("private-prompt-must-not-leak");
    expect(text).toContain("[REDACTED]");
  });
});
