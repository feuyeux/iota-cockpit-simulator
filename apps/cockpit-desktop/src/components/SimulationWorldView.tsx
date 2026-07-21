import { useMemo } from "react";
import { AlertCircle, Flame, RadioTower, Siren, Thermometer, User, Zap } from "lucide-react";
import type { DeviceState, HumanState, SimulationModel } from "../types/simulation";
import { CABIN_ZONES, getZoneLayout } from "../config/cabinLayout";
import { useI18n } from "../i18n";
import {
  actionStatusLabel,
  alertLabel,
  commandLabel,
  eventLabel,
  lifecycleLabel
} from "../utils/domainPresentation";
import type { Locale } from "../i18n";

/// Top-down floor-plan rendering of the current WorldSnapshot.
///
/// Design goals (per redesign request):
/// 1. Spatial view (A1): humans/devices are placed on a floor plan derived
///    from their `location` label via config/cabinLayout, instead of a flat
///    stat list. Zones render as rooms; environment state (smoke, lighting,
///    fire, alarm) renders as spatial overlays instead of numbers only.
/// 2. Causal traceability (B): the most recent SimulationEvent/ActionResult
///    that names a target entity is used to highlight that entity's marker
///    on the map, so the operator can see *which* part of the world model
///    just changed and *why* (event/action label surfaced in a tooltip-like
///    badge next to the marker).
interface LastEffect {
  targetId: string;
  label: string;
  tick: number;
}

/// Derive the most recent world-affecting cause, so it can be highlighted on
/// the map. Prefers the latest applied action result (clear causal action ->
/// effect), falling back to the latest event carrying a target.
function useLastEffect(model: SimulationModel, locale: Locale): LastEffect | undefined {
  return useMemo(() => {
    const latestAction = model.actionResults[0];
    if (latestAction && latestAction.request.target) {
      return {
        targetId: latestAction.request.target,
        label: `${commandLabel(latestAction.request.command, locale)} (${actionStatusLabel(latestAction.status, locale)})`,
        tick: latestAction.tick
      };
    }
    const latestEvent = model.events.find((event) => Boolean(event.payload.target));
    if (latestEvent && latestEvent.payload.target) {
      return {
        targetId: latestEvent.payload.target,
        label: eventLabel(latestEvent.eventType, locale),
        tick: latestEvent.tick
      };
    }
    return undefined;
  }, [locale, model.actionResults, model.events]);
}

function zoneIdForLocation(locationLabel: string | undefined): string {
  const zone = getZoneLayout(locationLabel);
  return zone.id === "__unknown__" ? "cabin" : zone.id;
}

function HumanMarker({
  human,
  highlighted
}: {
  human: HumanState;
  highlighted: LastEffect | undefined;
}) {
  const { t } = useI18n();
  const isHighlighted = highlighted?.targetId === human.id;
  return (
    <article
      className={`min-w-0 rounded border p-1.5 ${
        isHighlighted
          ? "border-amber-300 bg-amber-950/70 text-amber-100"
          : "border-emerald-700/60 bg-zinc-950/85 text-emerald-100"
      }`}
      data-testid={`marker-human-${human.id}`}
      title={`${human.persona.name} · ${human.persona.role} · ${human.location}`}
    >
      <div className="flex min-w-0 items-center gap-1.5">
        <User className="h-3.5 w-3.5 shrink-0" />
        <span className="min-w-0 flex-1 truncate text-xs font-medium">{human.persona.name}</span>
        <span className="max-w-16 truncate text-[10px] text-zinc-400">{human.persona.role}</span>
      </div>
      <dl className="mt-1 grid grid-cols-3 gap-1 text-[10px]">
        <div><dt className="text-zinc-500">{t("stress")}</dt><dd className="text-rose-200">{Math.round(human.stress * 100)}%</dd></div>
        <div><dt className="text-zinc-500">{t("attention")}</dt><dd className="text-cyan-200">{Math.round(human.attention * 100)}%</dd></div>
        <div><dt className="text-zinc-500">{t("health")}</dt><dd className="text-emerald-200">{Math.round(human.health * 100)}%</dd></div>
      </dl>
      {isHighlighted ? (
        <div className="mt-1 truncate border-t border-amber-300/30 pt-1 text-[10px] font-medium text-amber-200" title={highlighted.label}>
          t{highlighted.tick} · {highlighted.label}
        </div>
      ) : null}
    </article>
  );
}

function DeviceMarker({
  device,
  highlighted
}: {
  device: DeviceState;
  highlighted: LastEffect | undefined;
}) {
  const { locale, t } = useI18n();
  const isHighlighted = highlighted?.targetId === device.id;
  const faulted = device.faults.length > 0;
  return (
    <article
      className={`min-w-0 rounded border p-1.5 ${
        isHighlighted
          ? "border-amber-300 bg-amber-950/70 text-amber-100"
          : faulted
            ? "border-red-500/70 bg-red-950/55 text-red-100"
            : "border-cyan-700/60 bg-zinc-950/85 text-cyan-100"
      }`}
      data-testid={`marker-device-${device.id}`}
      title={`${device.id} · ${lifecycleLabel(device.lifecycle, locale)} · ${t("health")} ${(device.health * 100).toFixed(0)}% · ${device.capabilities.join(", ")}`}
    >
      <div className="flex min-w-0 items-center gap-1.5">
        {faulted ? <Zap className="h-3.5 w-3.5 shrink-0" /> : <RadioTower className="h-3.5 w-3.5 shrink-0" />}
        <span className="min-w-0 flex-1 truncate font-mono text-xs font-medium">{device.id}</span>
        <span className="shrink-0 text-[10px] text-zinc-400">{Math.round(device.health * 100)}%</span>
      </div>
      <div className="mt-1 grid grid-cols-[minmax(0,1fr)_auto] gap-1 text-[10px]">
        <span className="truncate text-zinc-400">{lifecycleLabel(device.lifecycle, locale)}</span>
        <span className="text-zinc-500">{device.capabilities.length} {t("deviceCapabilities")}</span>
      </div>
      {isHighlighted ? (
        <div className="mt-1 truncate border-t border-amber-300/30 pt-1 text-[10px] font-medium text-amber-200" title={highlighted.label}>
          t{highlighted.tick} · {highlighted.label}
        </div>
      ) : null}
    </article>
  );
}

export function SimulationWorldView({ model }: { model: SimulationModel }) {
  const { locale, t } = useI18n();
  const snapshot = model.snapshot;
  const observations = model.observations;
  const latestObservation = observations[0];
  const sensorDegraded = latestObservation?.quality.degraded ?? false;
  const humans = snapshot?.humans ?? [];
  const devices = snapshot?.devices ?? [];
  const lastEffect = useLastEffect(model, locale);

  const smokeDensity = snapshot?.environment.smokeDensity ?? 0;
  const fireActive = snapshot?.environment.fireActive ?? false;
  const alarmActive = snapshot?.alarm.active ?? false;
  const visibility = snapshot?.environment.visibility ?? 1;
  const lightingLux = snapshot?.environment.lightingLux;
  const systems = snapshot?.cockpitSystems;
  const systemRows = systems ? [
    [t("cooling"), systems.climate.coolingActive],
    [t("defog"), systems.climate.defogActive],
    [t("seatVentilation"), systems.climate.seatVentilationActive],
    [t("fatigueIntervention"), systems.driverAssistance.fatigueInterventionActive],
    [t("takeover"), systems.driverAssistance.takeoverAcknowledged],
    [t("takeoverHmi"), systems.driverAssistance.takeoverHmiActive],
    [t("childProtection"), systems.occupantCare.childProtectionActive],
    [t("medicalResponse"), systems.occupantCare.medicalResponseActive],
    [t("emergencyContacted"), systems.occupantCare.emergencyContacted],
    [t("guardianNotified"), systems.occupantCare.guardianNotified],
    [t("remoteUnlock"), systems.occupantCare.remoteUnlockRequested],
    [t("privacyMode"), systems.experience.privacyModeActive],
    [t("chargingPlan"), systems.experience.chargingPlanAccepted],
    [t("mediaIsolation"), systems.experience.mediaSessionsIsolated],
    [t("profileIsolation"), systems.experience.occupantProfilesIsolated],
    [t("emergencyRoute"), systems.mobility.emergencyRouteActive],
    [t("chargingRoute"), systems.mobility.chargingRouteActive],
    [t("chargerService"), systems.mobility.chargerServiceConnected],
    [t("emergencyCall"), systems.connectivity.emergencyCallActive],
    [t("remoteServiceIsolation"), systems.connectivity.remoteServicesIsolated],
    [t("trustedLocalAlert"), systems.connectivity.trustedLocalAlertActive],
    [t("cyberSafeMode"), systems.cybersecurity.safeModeActive],
    [t("networkIsolation"), systems.cybersecurity.networkIsolated],
    [t("identityVerified"), systems.cybersecurity.identityVerified]
  ] as const : [];
  const zoneLabels: Record<string, string> = {
    cockpit: t("cockpitZone"),
    "rear-left": t("rearLeft"),
    "rear-right": t("rearRight"),
    cabin: t("cabin")
  };

  return (
    <section className="world-view flex min-h-0 min-w-0 flex-col overflow-hidden rounded-xl border border-zinc-800/90 bg-zinc-900/60 backdrop-blur-md shadow-sm">
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-zinc-800/80 bg-zinc-900/80 px-3.5 py-2 text-xs font-semibold text-zinc-100 shrink-0">
        <span className="tracking-wide">{t("world")}</span>
        <div className="flex items-center gap-2">
          {fireActive ? (
            <span className="flex items-center gap-1 text-xs text-orange-300">
              <Flame className="h-3 w-3" />
              {t("fireActive")}
            </span>
          ) : null}
          {alarmActive && (
            <span className="flex items-center gap-1 text-xs text-red-300">
              <Siren className="h-3 w-3" />
              {t("alarm")}
            </span>
          )}
          {sensorDegraded && (
            <span className="flex items-center gap-1 text-xs text-amber-300">
              <AlertCircle className="h-3 w-3" />
              {t("sensorDegraded")}
            </span>
          )}
          <span className="text-xs text-zinc-400">{t("groundTruthHidden")}</span>
        </div>
      </div>
      {/* Main spatial cabin floor plan view takes full width */}
      <div className="relative min-h-0 flex-1 overflow-hidden p-1.5">
        <div className="absolute inset-1.5 overflow-hidden rounded border border-zinc-800/90 bg-zinc-950/90 shadow-inner" data-testid="floor-plan">
          {/* Environment overlays: visibility haze, smoke, fire, alarm tint */}
          <div
            className="pointer-events-none absolute inset-0 bg-zinc-300/10 transition-opacity"
            style={{ opacity: 1 - visibility }}
            data-testid="visibility-overlay"
          />
          {smokeDensity > 0 ? (
            <div
              className="pointer-events-none absolute inset-0 bg-zinc-400/20 transition-opacity"
              style={{ opacity: Math.min(0.85, smokeDensity) }}
              data-testid="smoke-overlay"
            />
          ) : null}
          {fireActive ? <div className="pointer-events-none absolute inset-0 animate-pulse bg-orange-600/15" /> : null}
          {alarmActive ? <div className="pointer-events-none absolute inset-0 animate-pulse border-2 border-red-500/50" /> : null}

          {CABIN_ZONES.map((zone) => {
            const zoneHumans = humans
              .filter((human) => zoneIdForLocation(human.location) === zone.id)
              .sort((left, right) => left.persona.name.localeCompare(right.persona.name));
            const zoneDevices = devices
              .filter((device) => {
                const deviceZone = getZoneLayout(device.id);
                return (deviceZone.id === "__unknown__" ? "cockpit" : deviceZone.id) === zone.id;
              })
              .sort((left, right) => left.id.localeCompare(right.id));
            const entityCount = zoneHumans.length + zoneDevices.length;

            return (
              <section
                key={zone.id}
                className="absolute flex min-h-0 flex-col overflow-hidden rounded border border-zinc-700/60 bg-zinc-900/40 p-1.5 backdrop-blur-[2px]"
                data-testid={`cabin-zone-${zone.id}`}
                style={{
                  left: `${zone.x}%`,
                  top: `${zone.y}%`,
                  width: `${zone.width}%`,
                  height: `${zone.height}%`
                }}
              >
                <header className="mb-1 flex shrink-0 items-center justify-between gap-2 border-b border-zinc-700/60 pb-1">
                  <span className="truncate text-[10px] font-medium uppercase tracking-wide text-zinc-400">
                    {zoneLabels[zone.id] ?? zone.label}
                  </span>
                  <span className="rounded bg-zinc-800 px-1 text-[10px] text-zinc-400">{entityCount}</span>
                </header>
                <div className="grid min-h-0 flex-1 content-start grid-cols-1 gap-1 overflow-y-auto pr-0.5">
                  {zoneHumans.map((human) => <HumanMarker key={human.id} human={human} highlighted={lastEffect} />)}
                  {zoneDevices.map((device) => <DeviceMarker key={device.id} device={device} highlighted={lastEffect} />)}
                  {entityCount === 0 ? (
                    <div className="flex min-h-10 items-center justify-center rounded border border-dashed border-zinc-800/80 py-1 text-xs text-zinc-600">
                      {t("emptySeat")}
                    </div>
                  ) : null}
                </div>
              </section>
            );
          })}
        </div>
      </div>

      {/* Legend & Details panel moved BELOW the cabin floor plan */}
      <aside className="shrink-0 max-h-40 min-h-0 overflow-y-auto border-t border-zinc-800/80 bg-zinc-950/80 p-2 px-3 text-xs text-zinc-300">
        <div className="grid grid-cols-2 gap-2.5 md:grid-cols-4">
          {/* Col 1: Legend & Sensor Quality */}
          <div className="space-y-2">
            <div>
              <div className="mb-1.5 text-[11px] font-semibold text-zinc-400">{t("legend")}</div>
              <div className="space-y-1 text-[10px] text-zinc-400">
                <div className="flex items-center gap-1.5">
                  <User className="h-3 w-3 text-emerald-300" /> {t("human")}
                </div>
                <div className="flex items-center gap-1.5">
                  <RadioTower className="h-3 w-3 text-cyan-300" /> {t("device")}
                </div>
                <div className="flex items-center gap-1.5">
                  <Zap className="h-3 w-3 text-red-300" /> {t("faultedDevice")}
                </div>
                <div className="flex items-center gap-1.5 text-amber-300">
                  <AlertCircle className="h-3 w-3" /> {t("lastAffected")}
                </div>
              </div>
            </div>

            {latestObservation && (
              <div className="space-y-0.5 border-t border-zinc-800/80 pt-1.5 text-[11px]">
                <div className="text-[10px] text-zinc-500">{t("sensorQuality")}</div>
                <div>{t("visibility")}: {(latestObservation.quality.visibilityQuality * 100).toFixed(0)}%</div>
                <div>{t("audio")}: {(latestObservation.quality.audioQuality * 100).toFixed(0)}%</div>
                <div>{t("confidence")}: {(latestObservation.quality.confidence * 100).toFixed(0)}%</div>
              </div>
            )}
          </div>

          {/* Col 2: Environment & Last Effect */}
          <div className="space-y-2">
            {snapshot ? (
              <div className="space-y-0.5 text-[11px]">
                <div className="flex items-center gap-1 text-[10px] text-zinc-400"><Thermometer className="h-3 w-3" />{t("outer")}</div>
                <div>{t("externalTemperature")}: {snapshot.outerEnvironment.externalTemperatureC.toFixed(1)}°C</div>
                <div>{t("wind")}: {snapshot.outerEnvironment.windSpeedKmh.toFixed(1)} km/h</div>
                <div className="mt-1 flex items-center gap-1 text-[10px] text-zinc-400"><Thermometer className="h-3 w-3" />{t("cabin")}</div>
                <div>{t("temperature")}: {snapshot.environment.temperatureC.toFixed(1)}°C</div>
                <div>{t("smoke")}: {snapshot.environment.smokeDensity.toFixed(2)}</div>
                {lightingLux !== undefined ? <div>{t("lighting")}: {lightingLux.toFixed(0)} lux</div> : null}
              </div>
            ) : null}

            {lastEffect && (
              <div className="space-y-0.5 border-t border-zinc-800/80 pt-1.5 text-[11px]">
                <div className="text-[10px] text-zinc-400">{t("lastEffect")}</div>
                <div className="text-amber-300">{lastEffect.label}</div>
                <div className="text-zinc-500">
                  {t("onTarget")} <span className="text-zinc-300">{lastEffect.targetId}</span> · t{lastEffect.tick}
                </div>
              </div>
            )}
          </div>

          {/* Col 3: Active Alerts & System Status */}
          <div className="space-y-2">
            {latestObservation && (
              <div className="text-[11px]">
                <div className="mb-1 text-[10px] text-zinc-400">{t("activeAlerts")}</div>
                {latestObservation.alerts.length > 0 ? (
                  <div className="space-y-1">
                    {latestObservation.alerts.map((alert) => (
                      <div key={alert} className="flex items-start gap-1.5 text-amber-300" title={alert}>
                        <AlertCircle className="mt-0.5 h-3 w-3 shrink-0" />
                        <span>{alertLabel(alert, locale)}</span>
                      </div>
                    ))}
                  </div>
                ) : (
                  <div className="text-zinc-600">{t("noAlerts")}</div>
                )}
              </div>
            )}

            {systems ? (
              <div className="border-t border-zinc-800/80 pt-1.5 text-[10px]">
                <div className="mb-1 text-xs text-zinc-400">{t("systemStatus")}</div>
                <div className="max-h-20 space-y-0.5 overflow-y-auto">
                  {systemRows.map(([label, active]) => (
                    <div key={label} className="flex items-center justify-between gap-2">
                      <span className="truncate text-zinc-500">{label}</span>
                      <span className={active ? "text-emerald-300" : "text-zinc-700"}>
                        {active ? t("active") : t("inactive")}
                      </span>
                    </div>
                  ))}
                  {systems.climate.comfortTargetC != null ? (
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-zinc-500">{t("comfortTarget")}</span>
                      <span className="text-cyan-300">{systems.climate.comfortTargetC.toFixed(1)}°C</span>
                    </div>
                  ) : null}
                </div>
              </div>
            ) : null}
          </div>

          {/* Col 4: Device Inventory & Empty Status */}
          <div className="space-y-2">
            {devices.length > 0 ? (
              <div className="text-[10px]">
                <div className="mb-1 text-xs text-zinc-400">{t("deviceInventory")}</div>
                <div className="max-h-24 space-y-1 overflow-y-auto">
                  {devices.map((device) => (
                    <div key={device.id} className="border-l border-zinc-800 pl-2">
                      <div className="flex items-center justify-between gap-2">
                        <span className="truncate font-mono text-cyan-200" title={device.id}>{device.id}</span>
                        <span className="shrink-0 text-zinc-600">{Math.round(device.health * 100)}%</span>
                      </div>
                      <div className="truncate text-zinc-600" title={device.capabilities.join(", ")}>
                        {lifecycleLabel(device.lifecycle, locale)} · {device.capabilities.length} {t("deviceCapabilities")}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            ) : null}

            {humans.length === 0 && devices.length === 0 && (
              <div className="text-xs text-zinc-500">
                {t("noEntities")}
              </div>
            )}
          </div>
        </div>
      </aside>
    </section>
  );
}
