"""Validate thermoregulation direction and humidity coupling without fitting cohort data.

The simulator study points are manually digitized from Figure 1 of Che Muhamed et al.
(DOI 10.1080/23328940.2016.1182669). They are intentionally used only for
ordinal checks because the protocol was exercise at 70% VO2max, not a resting
vehicle occupant. The passive seated protocol from Malcolm et al. (DOI
10.3389/fphys.2018.00585) supplies the vehicle-relevant hot-vs-moderate checks.
"""
from __future__ import annotations

import hashlib
import json
import math
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CORE = ROOT / "crates" / "cockpit-world" / "src" / "digital_twin.rs"
PROFILE = ROOT / "calibration" / "profiles" / "human-heat-stress-validation-v1.json"
SIMULATOR_FIGURE_SHA256 = "0775a043aa014d33bbd38a1f8c0919c84cb941e213040c6c181ec99905219f23"

# Approximate means read from the published 659 x 1255 Figure 1. The source
# error bars are much larger than the +/-0.10 C digitization allowance. These
# values are evidence records, never fit targets.
DIGITIZED_60_MIN_C = {
    "relativeHumidityPct": [23, 43, 52, 61, 71],
    "rectalTemperatureC": [38.76, 39.02, 39.12, 39.22, 39.49],
    "meanSkinTemperatureC": [31.70, 32.10, 32.49, 32.87, 33.21],
    "meanBodyTemperatureC": [37.27, 37.56, 37.72, 37.87, 38.16],
}


def rust_default(source: str, name: str) -> float:
    match = re.search(rf"\b{name}:\s*([0-9][0-9_]*(?:\.[0-9_]+)?)", source)
    if match is None:
        raise SystemExit(f"runtime physiology parameter missing: {name}")
    return float(match.group(1).replace("_", ""))


def saturation_vapour_pressure_kpa(temperature_c: float) -> float:
    return 0.61094 * math.exp(17.625 * temperature_c / (temperature_c + 243.04))


def simulate(
    parameters: dict[str, float],
    ambient_c: float,
    relative_humidity_pct: float,
    metabolic_heat_w: float,
    duration_s: int = 3600,
) -> dict[str, float]:
    core_c = 37.0
    skin_c = 33.7
    peak_evaporation_w = 0.0
    for _ in range(duration_s):
        core_skin_w = (
            parameters["core_skin_conductance_w_m2_k"]
            * parameters["body_surface_area_m2"]
            * (core_c - skin_c)
        )
        sensible_w = (
            parameters["clothed_heat_transfer_w_m2_k"]
            * parameters["body_surface_area_m2"]
            * (skin_c - ambient_c)
        )
        respiratory_w = parameters["basal_respiratory_heat_w"] + 0.15 * abs(
            ambient_c - 22.0
        )
        sweat_drive_w = parameters["body_surface_area_m2"] * (
            parameters["core_sweat_response_w_m2_k"] * max(0.0, core_c - 37.0)
            + parameters["skin_sweat_response_w_m2_k"] * max(0.0, skin_c - 33.7)
        )
        evaporation_capacity_w = (
            parameters["evaporative_heat_transfer_ratio_per_kpa"]
            * parameters["clothed_heat_transfer_w_m2_k"]
            * parameters["body_surface_area_m2"]
            * max(
                0.0,
                saturation_vapour_pressure_kpa(skin_c)
                - relative_humidity_pct
                / 100.0
                * saturation_vapour_pressure_kpa(ambient_c),
            )
        )
        evaporation_w = min(sweat_drive_w, evaporation_capacity_w)
        peak_evaporation_w = max(peak_evaporation_w, evaporation_w)
        core_c = min(
            43.0,
            max(
                32.0,
                core_c
                + (metabolic_heat_w - core_skin_w - respiratory_w)
                / parameters["core_heat_capacity_j_k"],
            ),
        )
        skin_c = min(
            42.0,
            max(
                15.0,
                skin_c
                + (core_skin_w - sensible_w - evaporation_w)
                / parameters["skin_heat_capacity_j_k"],
            ),
        )
    return {
        "coreTemperatureC": core_c,
        "skinTemperatureC": skin_c,
        "peakEvaporativeHeatLossW": peak_evaporation_w,
    }


def strictly_increasing(values: list[float]) -> bool:
    return all(left < right for left, right in zip(values, values[1:]))


def main() -> None:
    source = CORE.read_text(encoding="utf-8")
    names = (
        "body_surface_area_m2",
        "core_heat_capacity_j_k",
        "skin_heat_capacity_j_k",
        "core_skin_conductance_w_m2_k",
        "clothed_heat_transfer_w_m2_k",
        "resting_metabolic_heat_w",
        "basal_respiratory_heat_w",
        "evaporative_heat_transfer_ratio_per_kpa",
        "core_sweat_response_w_m2_k",
        "skin_sweat_response_w_m2_k",
    )
    parameters = {name: rust_default(source, name) for name in names}
    moderate = simulate(parameters, 21.2, 41.9, parameters["resting_metabolic_heat_w"])
    baseline = simulate(parameters, 22.0, 45.0, parameters["resting_metabolic_heat_w"])
    hot = simulate(parameters, 39.6, 50.8, parameters["resting_metabolic_heat_w"])

    # Elevated metabolism is a mechanism probe only. It is deliberately not
    # described as a reproduction or fit of the 70% VO2max cohort.
    humidity_probe = [
        simulate(parameters, 31.0, rh, 350.0) for rh in DIGITIZED_60_MIN_C["relativeHumidityPct"]
    ]
    probe_core = [row["coreTemperatureC"] for row in humidity_probe]
    probe_skin = [row["skinTemperatureC"] for row in humidity_probe]
    checks = {
        "restingCoreDriftUnder005C": abs(baseline["coreTemperatureC"] - 37.0) < 0.05,
        "restingSkinDriftUnder010C": abs(baseline["skinTemperatureC"] - 33.7) < 0.1,
        "passiveHotCoreAboveModerate": hot["coreTemperatureC"] > moderate["coreTemperatureC"],
        "passiveHotSkinAboveModerate": hot["skinTemperatureC"] > moderate["skinTemperatureC"],
        "higherHumidityRaisesProbeCoreEndpoint": strictly_increasing(probe_core),
        "higherHumidityRaisesProbeSkinEndpoint": strictly_increasing(probe_skin),
    }
    if not all(checks.values()):
        failed = [name for name, passed in checks.items() if not passed]
        raise SystemExit(f"human heat-stress validation failed: {', '.join(failed)}")

    profile = {
        "schemaVersion": 1,
        "profileId": "human-heat-stress-validation-v1",
        "modelVersion": "cockpit-multiphysics-4",
        "claimLevel": "experimentally-anchored-not-fitted",
        "sources": {
            "humidityExercise": {
                "source": "Che Muhamed et al., Temperature 3(3), 2016",
                "doi": "10.1080/23328940.2016.1182669",
                "pmcid": "PMC5079215",
                "license": "CC BY-NC",
                "participants": "11 trained males",
                "protocol": "60 min at 31 C, 70% VO2max, 23-71% RH",
                "figure": 1,
                "figureSha256": SIMULATOR_FIGURE_SHA256,
                "sourceImageRedistributed": False,
                "digitizationUncertaintyC": 0.10,
                "digitized60MinMeans": DIGITIZED_60_MIN_C,
                "use": "ordinal humidity-response evidence only; no resting-occupant parameter fit",
            },
            "passiveSeatedHeat": {
                "source": "Malcolm et al., Frontiers in Physiology 9:585, 2018",
                "doi": "10.3389/fphys.2018.00585",
                "license": "CC BY 4.0",
                "participants": "41 healthy active males",
                "protocol": "randomized crossover; 60 min seated rest at 39.6 C/50.8% RH or 21.2 C/41.9% RH",
                "observations": "core greater in heat from 30 min onward; skin greater at every post-baseline point; hot sweat rate 0.56 +/- 0.38 L/h",
                "use": "vehicle-relevant directional external check; published article does not expose a numeric table for temperature fitting",
            },
        },
        "runtimeParameters": parameters,
        "validation": {
            "accepted": True,
            "acceptanceScope": "direction, resting stability, and humidity-coupling mechanism only",
            "parameterFitPerformed": False,
            "moderateSeated60Min": moderate,
            "resting22C60Min": baseline,
            "hotSeated60Min": hot,
            "humidityMechanismProbe": {
                "ambientC": 31.0,
                "metabolicHeatW": 350.0,
                "notACohortFit": True,
                "relativeHumidityPct": DIGITIZED_60_MIN_C["relativeHumidityPct"],
                "outputs": humidity_probe,
            },
            "checks": checks,
        },
        "limitations": [
            "simulator endpoints were graph-digitized and are not raw observations",
            "exercise metabolism and clothing were not fitted",
            "passive study measured one thigh site rather than whole-body mean skin temperature",
            "sweat feedback and Lewis capacity parameters remain engineering values",
            "individual age, sex, acclimation, hydration, clothing, and activity are not modeled",
        ],
    }
    PROFILE.parent.mkdir(parents=True, exist_ok=True)
    PROFILE.write_text(json.dumps(profile, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(
        "human_heat_stress=accepted_directional "
        f"rest_core_c={baseline['coreTemperatureC']:.6f} "
        f"rest_skin_c={baseline['skinTemperatureC']:.6f} "
        f"passive_hot_core_c={hot['coreTemperatureC']:.6f} "
        f"passive_hot_skin_c={hot['skinTemperatureC']:.6f} "
        "humidity_order=verified parameter_fit=False"
    )


if __name__ == "__main__":
    main()
