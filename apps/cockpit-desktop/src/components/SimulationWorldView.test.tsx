import { describe, it, expect, afterEach } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { SimulationWorldView } from "./SimulationWorldView";
import { initialSimulationModel } from "../state/simulationReducer";
import type { ActionResult, HumanState, Observation, SimulationModel } from "../types/simulation";
import { I18nProvider } from "../i18n";

// Component tests render into jsdom via react-dom/client so no extra test
// dependency (react-testing-library) is required.

let container: HTMLDivElement | null = null;
let root: Root | null = null;

function render(model: SimulationModel) {
  window.localStorage.setItem("cockpit:locale", "en-US");
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  act(() => {
    root!.render(
      <I18nProvider>
        <SimulationWorldView model={model} />
      </I18nProvider>
    );
  });
  return container;
}

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  container?.remove();
  container = null;
  root = null;
});

function observation(degraded: boolean): Observation {
  return {
    observationId: "obs-1",
    runId: "run-1",
    agentId: "agent-1",
    sensorId: "sensor-1",
    observedTick: 1,
    deliveredTick: 1,
    visibleEntities: [],
    alerts: [],
    actionResults: [],
    confidence: 0.5,
    quality: {
      visibilityQuality: 0.4,
      audioQuality: 0.6,
      confidence: 0.5,
      degraded,
    },
  } as Observation;
}

describe("SimulationWorldView", () => {
  it("hides Ground Truth and shows no degradation banner by default", () => {
    const el = render(initialSimulationModel);
    expect(el.textContent).toContain("Ground Truth hidden");
    expect(el.textContent).not.toContain("Sensor degraded");
  });

  it("shows the degradation banner and sensor quality when degraded", () => {
    const el = render({
      ...initialSimulationModel,
      observations: [observation(true)],
    });
    expect(el.textContent).toContain("Sensor degraded");
    expect(el.textContent).toContain("Visibility: 40%");
    expect(el.textContent).toContain("Confidence: 50%");
  });

  it("renders humans and devices from the snapshot", () => {
    const el = render({
      ...initialSimulationModel,
      snapshot: {
        runId: "run-1",
        tick: 1,
        simTimeMs: 100,
        version: 1,
        outerEnvironment: {
          externalTemperatureC: 20,
          altitudeM: 0,
          windSpeedKmh: 5,
          precipitation: 0,
          threatActive: false,
        },
        environment: {
          temperatureC: 22,
          humidityPct: 45,
          visibility: 1,
          smokeDensity: 0,
          lightingLux: 400,
          noiseDb: 42,
          fireActive: false,
        },
        humans: [
          {
            id: "pilot-1",
            persona: {
              name: "Alex",
              role: "pilot",
              background: "",
              traits: {
                openness: 0.5,
                conscientiousness: 0.8,
                extraversion: 0.4,
                agreeableness: 0.5,
                neuroticism: 0.3,
              },
              relationships: [],
            },
            needs: { comfort: 1, safety: 1, social: 1 },
            stress: 0.1,
            fatigue: 0,
            health: 1,
            attention: 0.9,
            location: "cockpit",
            goal: "maintain safe cockpit state",
            shortTermMemory: [],
            longTermMemory: [],
          },
        ],
        devices: [
          {
            id: "engine-1",
            health: 1,
            powerState: "powered",
            lifecycle: "Normal",
            faults: [],
            capabilities: ["shutdown"],
            shutdown: false,
          },
        ],
        alarm: { active: false, volumeDb: 0 },
      },
    });
    expect(el.textContent).toContain("engine-1");
    expect(el.textContent).toContain("Alex");
    expect(el.textContent).toContain("Device inventory");
    expect(el.textContent).toContain("1 capabilities");
  });

  function human(overrides: Partial<HumanState> = {}): HumanState {
    return {
      id: "pilot-1",
      persona: {
        name: "Alex",
        role: "pilot",
        background: "",
        traits: {
          openness: 0.5,
          conscientiousness: 0.8,
          extraversion: 0.4,
          agreeableness: 0.5,
          neuroticism: 0.3,
        },
        relationships: [],
      },
      needs: { comfort: 1, safety: 1, social: 1 },
      stress: 0.1,
      fatigue: 0,
      health: 1,
      attention: 0.9,
      location: "cockpit",
      goal: "maintain safe cockpit state",
      shortTermMemory: [],
      longTermMemory: [],
      ...overrides,
    };
  }

  function snapshotWith(overrides: {
    humans?: HumanState[];
    smokeDensity?: number;
    fireActive?: boolean;
    alarmActive?: boolean;
  }): SimulationModel["snapshot"] {
    return {
      runId: "run-1",
      tick: 5,
      simTimeMs: 500,
      version: 1,
      outerEnvironment: {
        externalTemperatureC: 20,
        altitudeM: 0,
        windSpeedKmh: 5,
        precipitation: 0,
        threatActive: false,
      },
      environment: {
        temperatureC: 22,
        humidityPct: 45,
        visibility: 1,
        smokeDensity: overrides.smokeDensity ?? 0,
        lightingLux: 400,
        noiseDb: 42,
        fireActive: overrides.fireActive ?? false,
      },
      humans: overrides.humans ?? [human()],
      devices: [
        {
          id: "engine-1",
          health: 1,
          powerState: "powered",
          lifecycle: "Normal",
          faults: [],
          capabilities: ["shutdown"],
          shutdown: false,
        },
      ],
      alarm: { active: overrides.alarmActive ?? false, volumeDb: 0 },
    };
  }

  it("renders the floor plan with zone rooms", () => {
    const el = render({ ...initialSimulationModel, snapshot: snapshotWith({}) });
    const floorPlan = el.querySelector('[data-testid="floor-plan"]');
    expect(el.querySelector(".world-view")).not.toBeNull();
    expect(floorPlan).not.toBeNull();
    expect(el.textContent).toContain("Cockpit");
    expect(el.textContent).toContain("Rear Left");
  });

  it("groups humans from different location labels into their matching zones", () => {
    const el = render({
      ...initialSimulationModel,
      snapshot: snapshotWith({
        humans: [
          human({ id: "pilot-1", location: "cockpit" }),
          human({ id: "passenger-1", location: "rear-left", persona: { ...human().persona, name: "Sam" } }),
        ],
      }),
    });
    const pilotMarker = el.querySelector('[data-testid="marker-human-pilot-1"]');
    const passengerMarker = el.querySelector('[data-testid="marker-human-passenger-1"]');
    const cockpitZone = el.querySelector('[data-testid="cabin-zone-cockpit"]');
    const rearLeftZone = el.querySelector('[data-testid="cabin-zone-rear-left"]');

    expect(cockpitZone?.contains(pilotMarker)).toBe(true);
    expect(rearLeftZone?.contains(passengerMarker)).toBe(true);
  });

  it("orders occupants before devices within a compact zone list", () => {
    const el = render({
      ...initialSimulationModel,
      snapshot: snapshotWith({
        humans: [
          human({ id: "sam-1", persona: { ...human().persona, name: "Sam" }, location: "cockpit" }),
          human({ id: "alex-1", persona: { ...human().persona, name: "Alex" }, location: "cockpit" }),
        ],
      }),
    });
    const cockpitZone = el.querySelector('[data-testid="cabin-zone-cockpit"]') as HTMLElement | null;
    const alex = cockpitZone?.querySelector('[data-testid="marker-human-alex-1"]') as HTMLElement | null;
    const sam = cockpitZone?.querySelector('[data-testid="marker-human-sam-1"]') as HTMLElement | null;
    const device = cockpitZone?.querySelector('[data-testid="marker-device-engine-1"]') as HTMLElement | null;

    expect(alex).not.toBeNull();
    expect(sam).not.toBeNull();
    expect(device).not.toBeNull();
    expect(cockpitZone!.textContent!.indexOf("Alex")).toBeLessThan(cockpitZone!.textContent!.indexOf("Sam"));
    expect(cockpitZone!.textContent!.indexOf("Sam")).toBeLessThan(cockpitZone!.textContent!.indexOf("engine-1"));
    expect(alex!.className).not.toContain("absolute");
  });

  it("shows smoke and fire overlays when present in the snapshot", () => {
    const el = render({
      ...initialSimulationModel,
      snapshot: snapshotWith({ smokeDensity: 0.6, fireActive: true }),
    });
    expect(el.querySelector('[data-testid="smoke-overlay"]')).not.toBeNull();
    expect(el.textContent).toContain("Fire active");
  });

  it("shows an alarm indicator in the header when the alarm is active", () => {
    const el = render({
      ...initialSimulationModel,
      snapshot: snapshotWith({ alarmActive: true }),
    });
    expect(el.textContent).toContain("Alarm");
  });

  it("shows localized active domain alerts", () => {
    const riskObservation = observation(false);
    riskObservation.alerts = ["ThermalComfortRisk", "CyberControlAnomaly"];
    const el = render({
      ...initialSimulationModel,
      observations: [riskObservation],
    });
    expect(el.textContent).toContain("Active alerts");
    expect(el.textContent).toContain("Thermal comfort risk");
    expect(el.textContent).toContain("Remote-control anomaly");
  });

  it("shows authoritative cockpit subsystem states", () => {
    const snapshot = snapshotWith({});
    snapshot!.cockpitSystems = {
      climate: {
        comfortTargetC: 25.5,
        coolingActive: true,
        defogActive: false,
        seatVentilationActive: true
      },
      driverAssistance: {
        fatigueInterventionActive: false,
        takeoverAcknowledged: true,
        takeoverHmiActive: true
      },
      occupantCare: {
        childProtectionActive: false,
        medicalResponseActive: false,
        emergencyContacted: false,
        guardianNotified: false,
        remoteUnlockRequested: false
      },
      experience: {
        privacyModeActive: true,
        chargingPlanAccepted: false,
        mediaSessionsIsolated: true,
        occupantProfilesIsolated: true
      },
      mobility: {
        emergencyRouteActive: false,
        chargingRouteActive: false,
        chargerServiceConnected: false
      },
      connectivity: {
        emergencyCallActive: false,
        remoteServicesIsolated: true,
        trustedLocalAlertActive: true
      },
      cybersecurity: { safeModeActive: true, networkIsolated: true, identityVerified: true }
    };
    const el = render({ ...initialSimulationModel, snapshot });
    expect(el.textContent).toContain("Cockpit system status");
    expect(el.textContent).toContain("Comfort coolingActive");
    expect(el.textContent).toContain("Seat ventilationActive");
    expect(el.textContent).toContain("Comfort target25.5°C");
    expect(el.textContent).toContain("Network isolationActive");
    expect(el.textContent).toContain("Device identity verifiedActive");
  });

  it("highlights the entity targeted by the most recent action result", () => {
    const actionResult: ActionResult = {
      request: {
        requestId: "req-1",
        agentId: "cockpit-agent",
        target: "engine-1",
        command: "engineShutdown",
        expectedStateVersion: 1,
        expiresAtTick: 10,
        correlationId: "corr-1",
      },
      status: "applied",
      runId: "run-1",
      tick: 5,
      correlationId: "corr-1",
    };
    const el = render({
      ...initialSimulationModel,
      snapshot: snapshotWith({}),
      actionResults: [actionResult],
    });
    expect(el.textContent).toContain("Last Effect");
    expect(el.textContent).toContain("Shut down engine (Applied)");
    const marker = el.querySelector('[data-testid="marker-device-engine-1"]');
    expect(marker?.textContent).toContain("t5");
  });
});
