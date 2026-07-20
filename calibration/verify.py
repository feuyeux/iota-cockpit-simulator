#!/usr/bin/env python3
"""Verify all physical-model evidence without compiling the Rust workspace."""

from __future__ import annotations

import csv
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CALIBRATION = ROOT / "calibration"
THERMAL_SHA256 = "9075e138317faa93be66891af8173dc9070e3782105d2f40f9f6f2273e89e777"
FIRE_SHA256 = "4957b94564cd338dca3098e849309e5ce442f3c8a5e6191375a42d92f2463a26"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def csv_rows(path: Path) -> int:
    with path.open(newline="", encoding="utf-8") as handle:
        return sum(1 for _ in csv.reader(handle)) - 1


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


def main() -> None:
    thermal = json.loads(
        (CALIBRATION / "profiles" / "mendeley-sedan-v1.json").read_text(encoding="utf-8")
    )
    fire = json.loads(
        (CALIBRATION / "profiles" / "nist-vehicle2-combustion-v1.json").read_text(
            encoding="utf-8"
        )
    )
    human = json.loads(
        (CALIBRATION / "profiles" / "human-heat-stress-validation-v1.json").read_text(
            encoding="utf-8"
        )
    )
    core = (
        ROOT / "crates" / "cockpit-world" / "src" / "digital_twin.rs"
    ).read_text(encoding="utf-8")
    generated_fire = (
        ROOT / "crates" / "cockpit-world" / "src" / "generated_vehicle_fire.rs"
    ).read_text(encoding="utf-8")
    world = (
        ROOT / "crates" / "cockpit-world" / "src" / "world.rs"
    ).read_text(encoding="utf-8")
    recording = (ROOT / "crates" / "cockpit-recording" / "src" / "lib.rs").read_text(
        encoding="utf-8"
    )

    require(
        sha256(CALIBRATION / "source" / "thermal-cabin-database.xlsx") == THERMAL_SHA256,
        "thermal source hash mismatch",
    )
    require(
        sha256(CALIBRATION / "source" / "nist-vehicle2-fire.csv") == FIRE_SHA256,
        "vehicle-fire source hash mismatch",
    )
    require(csv_rows(CALIBRATION / "datasets" / "mendeley-sedan-experiment-d.csv") == 1302,
            "thermal observation count mismatch")
    require(csv_rows(CALIBRATION / "datasets" / "nist-vehicle2-hrr-10s.csv") == 618,
            "vehicle-fire anchor count mismatch")
    require(thermal["validation"]["accepted"], "thermal profile is not accepted")
    require(
        thermal["validation"]["recursiveHoldout"]["rmseC"]
        < thermal["validation"]["persistenceHoldout"]["rmseC"],
        "thermal profile does not beat persistence",
    )
    require(fire["validation"]["accepted"], "vehicle-fire profile is not accepted")
    require(
        fire["validation"]["interpolationRmseKw"]
        < fire["validation"]["persistenceRmseKw"],
        "vehicle-fire profile does not beat persistence",
    )
    require(human["validation"]["accepted"], "human heat-stress profile is not accepted")
    require(not human["validation"]["parameterFitPerformed"],
            "human directional evidence must not be represented as a parameter fit")
    require(human["claimLevel"] == "experimentally-anchored-not-fitted",
            "human physiology claim level was promoted without fit evidence")
    require(
        human["sources"]["humidityExercise"]["figureSha256"]
        == "0775a043aa014d33bbd38a1f8c0919c84cb941e213040c6c181ec99905219f23",
        "human heat-stress figure provenance mismatch",
    )
    require(not human["sources"]["humidityExercise"]["sourceImageRedistributed"],
            "CC BY-NC source image must not be redistributed")
    require(all(human["validation"]["checks"].values()),
            "human heat-stress directional check failed")
    require(generated_fire.count("    (") == 618, "generated HRR lookup count mismatch")

    required_runtime = (
        'DIGITAL_TWIN_MODEL_VERSION: &str = "cockpit-multiphysics-4"',
        "measured_vehicle_fire_hrr_kw",
        "soot_yield_kg_kg: 0.0569",
        "carbon_monoxide_yield_kg_kg: 0.0590",
        "smoke_mass_extinction_m2_mg: 0.0087",
        "smoke_deposition_to_air_change_ratio: 1.3",
        "1_013.6 - 0.112 * altitude_m",
        "cohb_model_a_min: 241.0",
        "cohb_model_b_inv_mmhg: 1_421.0",
        "core_skin_conductance_w_m2_k: 16.0",
        "evaporative_heat_transfer_ratio_per_kpa: 16.5",
        "core_sweat_response_w_m2_k: 180.0",
        "skin_sweat_response_w_m2_k: 20.0",
        "advance_two_node_temperatures",
    )
    for marker in required_runtime:
        require(marker in core, f"runtime calibration marker missing: {marker}")
    forbidden_runtime = (
        "pub smoke_source_mg_s",
        "carbon_monoxide_source_ppm_s",
        "smoke_deposition_s",
        "cohb_uptake_pct_per_ppm_s",
        "cohb_clearance_time_s",
        "let body_area_m2 = 1.8",
        "let metabolic_w = 105.0",
    )
    for marker in forbidden_runtime:
        require(marker not in core, f"obsolete fixed rule remains: {marker}")

    scope = thermal["scope"]
    require(len(scope["calibrated"]) == 2, "calibrated claim tier mismatch")
    require(len(scope["externallyValidatedModels"]) == 2,
            "external-validation claim tier mismatch")
    require(len(scope["experimentallyAnchoredNotFitted"]) == 4,
            "empirical-anchor claim tier mismatch")
    require(len(scope["physicsBasedNotDatasetCalibrated"]) == 7,
            "unfitted-boundary claim tier mismatch")
    require("cockpit-world-snapshot-v6" in world, "snapshot hash domain mismatch")
    require("CURRENT_WORLD_MODEL_VERSION: u32 = 8" in recording,
            "world model version mismatch")
    require("CURRENT_RUNTIME_CONTRACT_VERSION: u32 = 6" in recording,
            "runtime contract version mismatch")

    print(
        "thermal=accepted observations=1302 "
        f"rmse_c={thermal['validation']['recursiveHoldout']['rmseC']:.6f}"
    )
    print(
        "vehicle_fire=accepted anchors=618 "
        f"rmse_kw={fire['validation']['interpolationRmseKw']:.6f}"
    )
    print(
        "human_heat_stress=directional humidity_order=verified "
        f"rest_core_c={human['validation']['resting22C60Min']['coreTemperatureC']:.6f} "
        "parameter_fit=False"
    )
    print("smoke=optics+nist_yield+vehicle_deposition co=vehicle_yield+field_cohb")
    print("pressure=vehicle_altitude_fit+parked_ach physiology=rest+passive+humidity_direction")
    print("claim_tiers=4 world_model=8 snapshot_hash=v6 all_evidence=verified")


if __name__ == "__main__":
    main()
