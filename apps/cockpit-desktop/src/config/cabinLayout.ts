/// Static floor-plan layout for the cockpit cabin, used by SimulationWorldView
/// to place humans/devices spatially instead of as a flat list.
///
/// The simulation core only models `location` as a free-text label (see
/// WorldSnapshot.humans[].location and scenario YAML `components.location`),
/// there is no coordinate system in cockpit-world. Rather than
/// changing the scenario schema/backend, we keep a small label -> layout
/// mapping here on the desktop side. Unknown labels fall back to a generic
/// slot so the view never breaks on new scenarios.
export interface ZoneLayout {
  id: string;
  label: string;
  /// Percentage-based box within the floor plan (0-100), so the view scales
  /// with any container size.
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface AnchorLayout {
  x: number;
  y: number;
}

export const CABIN_ZONES: ZoneLayout[] = [
  { id: "cockpit", label: "Cockpit", x: 2, y: 2, width: 47, height: 47 },
  { id: "rear-left", label: "Rear Left", x: 2, y: 51, width: 47, height: 47 },
  { id: "rear-right", label: "Rear Right", x: 51, y: 51, width: 47, height: 47 },
  { id: "cabin", label: "Cabin", x: 51, y: 2, width: 47, height: 47 }
];

const FALLBACK_ZONE: ZoneLayout = {
  id: "__unknown__",
  label: "Unplaced",
  x: 40,
  y: 40,
  width: 20,
  height: 20
};

/// Deterministic jitter so multiple occupants of the same zone don't render
/// exactly on top of each other. Derived from the entity id, not random, so
/// positions are stable across re-renders and ticks.
function stableOffset(id: string, spread: number): { dx: number; dy: number } {
  let hash = 0;
  for (let i = 0; i < id.length; i += 1) {
    hash = (hash * 31 + id.charCodeAt(i)) & 0xffffffff;
  }
  const angle = (Math.abs(hash) % 360) * (Math.PI / 180);
  return { dx: Math.cos(angle) * spread, dy: Math.sin(angle) * spread };
}

export function getZoneLayout(locationLabel: string | undefined): ZoneLayout {
  if (!locationLabel) return FALLBACK_ZONE;
  const normalized = locationLabel.trim().toLowerCase();
  return CABIN_ZONES.find((zone) => zone.id === normalized) ?? { ...FALLBACK_ZONE, label: locationLabel };
}

/// Resolve a stable (x, y) percentage position for an entity inside its zone,
/// so markers spread out instead of stacking.
export function getAnchorPosition(locationLabel: string | undefined, entityId: string): AnchorLayout {
  const zone = getZoneLayout(locationLabel);
  const { dx, dy } = stableOffset(entityId, Math.min(zone.width, zone.height) * 0.22);
  const centerX = zone.x + zone.width / 2;
  const centerY = zone.y + zone.height / 2;
  return {
    x: clampPercent(centerX + dx),
    y: clampPercent(centerY + dy)
  };
}

function clampPercent(value: number): number {
  return Math.min(96, Math.max(4, value));
}
