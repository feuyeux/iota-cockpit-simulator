import { describe, expect, it } from "vitest";
import { BENCHMARK_SCENARIOS, COCKPIT_DOMAINS, findBenchmarkScenarioByPath, localize } from "./scenarioCatalog";

describe("benchmark scenario catalog", () => {
  it("contains ten unique runnable scenario entries", () => {
    expect(BENCHMARK_SCENARIOS).toHaveLength(10);
    expect(new Set(BENCHMARK_SCENARIOS.map((scenario) => scenario.id)).size).toBe(10);
    expect(new Set(BENCHMARK_SCENARIOS.map((scenario) => scenario.path)).size).toBe(10);
    for (const scenario of BENCHMARK_SCENARIOS) {
      expect(scenario.path).toMatch(/^scenarios\/.+\.yaml$/);
      expect(scenario.coverage.length).toBeGreaterThanOrEqual(3);
      expect(scenario.domains.length).toBeGreaterThanOrEqual(4);
      expect(scenario.capability).toContain(".");
      expect(scenario.command.length).toBeGreaterThan(8);
      expect(scenario.target).toMatch(/-1$/);
      expect(scenario.evidenceEvent).toMatch(/^[A-Z]/);
      expect(scenario.deadlineTick).toBeGreaterThan(0);
      expect(scenario.deadlineTick).toBeLessThanOrEqual(30);
      expect(scenario.occupants).toBeGreaterThanOrEqual(2);
      expect(scenario.systems).toBeGreaterThanOrEqual(1);
    }
  });

  it("matches a catalog scenario from relative, absolute, and Windows-style Simulator paths", () => {
    const expected = BENCHMARK_SCENARIOS.find((scenario) => scenario.path === "scenarios/smoke-in-cockpit.yaml");

    expect(findBenchmarkScenarioByPath("scenarios/smoke-in-cockpit.yaml")).toBe(expected);
    expect(findBenchmarkScenarioByPath("/workspace/cockpit-simulator/scenarios/smoke-in-cockpit.yaml")).toBe(expected);
    expect(findBenchmarkScenarioByPath("C:\\workspace\\cockpit-simulator\\scenarios\\smoke-in-cockpit.yaml")).toBe(expected);
    expect(findBenchmarkScenarioByPath("/tmp/unknown.yaml")).toBeUndefined();
  });

  it("covers the complete cockpit domain taxonomy across the ten scenarios", () => {
    const covered = new Set(BENCHMARK_SCENARIOS.flatMap((scenario) => scenario.domains));
    expect(COCKPIT_DOMAINS).toHaveLength(14);
    expect(covered).toEqual(new Set(COCKPIT_DOMAINS.map((domain) => domain.id)));
    for (const domain of COCKPIT_DOMAINS) {
      expect(BENCHMARK_SCENARIOS.filter((scenario) => scenario.domains.includes(domain.id)).length)
        .toBeGreaterThanOrEqual(1);
    }
  });

  it("provides meaningful Chinese and English presentation text", () => {
    for (const scenario of BENCHMARK_SCENARIOS) {
      expect(localize(scenario.title, "zh-CN").length).toBeGreaterThan(4);
      expect(localize(scenario.title, "en-US").length).toBeGreaterThan(8);
      expect(localize(scenario.objective, "zh-CN")).not.toBe(localize(scenario.objective, "en-US"));
    }
  });
});
