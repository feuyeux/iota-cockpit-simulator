#!/usr/bin/env python3
"""Generate the measured NIST Vehicle2 combustion source prior.

The source CSV is immutable and hash-gated. A 10-second piecewise-linear HRR
curve is emitted for runtime use; all non-anchor one-second observations are
held out when reporting interpolation error.
"""

from __future__ import annotations

import csv
import hashlib
import json
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parent
SOURCE = ROOT / "source" / "nist-vehicle2-fire.csv"
DATASET = ROOT / "datasets" / "nist-vehicle2-hrr-10s.csv"
PROFILE = ROOT / "profiles" / "nist-vehicle2-combustion-v1.json"
RUST_OUTPUT = ROOT.parent / "crates" / "cockpit-world" / "src" / "generated_vehicle_fire.rs"
EXPECTED_SHA256 = "4957b94564cd338dca3098e849309e5ce442f3c8a5e6191375a42d92f2463a26"
ANCHOR_PERIOD_S = 10
PEAK_HRR_KW = 11_312.0
PEAK_TIME_S = 622.0
EFFECTIVE_HEAT_COMBUSTION_MJ_KG = 36.0
SOOT_YIELD_KG_KG = 0.0569
SOOT_YIELD_UNCERTAINTY_KG_KG = 0.0073
CO_YIELD_KG_KG = 0.0590
CO_YIELD_UNCERTAINTY_KG_KG = 0.0025


def rmse(predicted: list[float], actual: list[float]) -> float:
    return math.sqrt(sum((a - b) ** 2 for a, b in zip(predicted, actual)) / len(actual))


def main() -> None:
    digest = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    if digest != EXPECTED_SHA256:
        raise SystemExit(f"source hash mismatch: expected {EXPECTED_SHA256}, got {digest}")

    with SOURCE.open(newline="", encoding="utf-8") as handle:
        raw = list(csv.DictReader(handle))
    observations = [
        (int(float(row["Time (s)"])), max(float(row["Heat Release Rate (kW)"]), 0.0))
        for row in raw
        if float(row["Time (s)"]) >= 0.0
    ]
    by_time = dict(observations)
    anchors = [(time_s, hrr) for time_s, hrr in observations if time_s % ANCHOR_PERIOD_S == 0]
    if max(hrr for _, hrr in observations) != PEAK_HRR_KW:
        raise SystemExit("published peak HRR is absent from source")
    if max(observations, key=lambda item: item[1])[0] != int(PEAK_TIME_S):
        raise SystemExit("published peak time is absent from source")

    predicted: list[float] = []
    actual: list[float] = []
    persistence: list[float] = []
    final_anchor_s = anchors[-1][0]
    for time_s, hrr in observations:
        if time_s % ANCHOR_PERIOD_S == 0 or time_s > final_anchor_s:
            continue
        lower_s = time_s // ANCHOR_PERIOD_S * ANCHOR_PERIOD_S
        upper_s = lower_s + ANCHOR_PERIOD_S
        fraction = (time_s - lower_s) / ANCHOR_PERIOD_S
        predicted.append(by_time[lower_s] + (by_time[upper_s] - by_time[lower_s]) * fraction)
        persistence.append(by_time[lower_s])
        actual.append(hrr)

    DATASET.parent.mkdir(parents=True, exist_ok=True)
    with DATASET.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.writer(handle, lineterminator="\n")
        writer.writerow(("fireAgeS", "heatReleaseRateKw"))
        writer.writerows((time_s, f"{hrr:.6f}") for time_s, hrr in anchors)

    validation_rmse = rmse(predicted, actual)
    persistence_rmse = rmse(persistence, actual)
    profile = {
        "schemaVersion": 1,
        "profileId": "nist-vehicle2-combustion-v1",
        "source": {
            "databaseDoi": "10.18434/mds2-2314",
            "recordUrl": "https://www.nist.gov/el/fcd/design-fires-vehicles-pine-straw-bed/vehicle2",
            "csvUrl": "https://www.nist.gov/fcd-s3?path=/HRR/ASSET_FILES/DesignFires_Vehicles/data/1727100783_Vehicle2.csv",
            "csvSha256": EXPECTED_SHA256,
            "experiment": "2024 Vehicle2: full-scale 2007 ICE minivan on pine-straw bed",
            "license": "NIST public data (U.S. Government work; consult NIST terms)",
            "observations": len(observations),
        },
        "publishedMeasurements": {
            "peakHeatReleaseRateKw": PEAK_HRR_KW,
            "peakTimeS": PEAK_TIME_S,
            "effectiveHeatCombustionMjKg": EFFECTIVE_HEAT_COMBUSTION_MJ_KG,
            "sootYieldKgKg": SOOT_YIELD_KG_KG,
            "sootYieldExpandedUncertainty95PctKgKg": SOOT_YIELD_UNCERTAINTY_KG_KG,
            "carbonMonoxideYieldKgKg": CO_YIELD_KG_KG,
            "carbonMonoxideYieldExpandedUncertainty95PctKgKg": CO_YIELD_UNCERTAINTY_KG_KG,
        },
        "fit": {
            "method": "10-second piecewise-linear measured HRR lookup",
            "anchorPeriodS": ANCHOR_PERIOD_S,
            "anchors": len(anchors),
        },
        "validation": {
            "holdoutObservations": len(actual),
            "interpolationRmseKw": validation_rmse,
            "persistenceRmseKw": persistence_rmse,
            "acceptanceThresholdRmseKw": 60.0,
            "accepted": validation_rmse < 60.0 and validation_rmse < persistence_rmse,
        },
        "scope": {
            "calibrated": [
                "full-scale exterior ICE-vehicle HRR trajectory",
                "aggregate soot and carbon-monoxide yields per burned fuel mass",
            ],
            "notCalibrated": [
                "fraction of exterior fire effluent entering a closed passenger cabin",
                "other vehicle/fuel geometries",
                "cabin smoke deposition and inter-zone transport",
            ],
        },
    }
    PROFILE.write_text(json.dumps(profile, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    lines = [
        "//! Generated by calibration/calibrate_vehicle_fire.py; do not hand-edit.",
        "",
        "pub const NIST_VEHICLE2_HRR_KW: &[(f64, f64)] = &[",
    ]
    lines.extend(f"    ({time_s:.1f}, {hrr:.6f})," for time_s, hrr in anchors)
    lines.extend([
        "];",
        "",
        f"pub const NIST_VEHICLE2_SOURCE_SHA256: &str = \"{EXPECTED_SHA256}\";",
        "pub const NIST_VEHICLE2_PROFILE_ID: &str = \"nist-vehicle2-combustion-v1\";",
        "",
    ])
    RUST_OUTPUT.write_text("\n".join(lines), encoding="utf-8")
    print(
        f"source_sha256={digest} observations={len(observations)} anchors={len(anchors)} "
        f"holdout={len(actual)} interpolation_rmse_kw={validation_rmse:.6f} "
        f"persistence_rmse_kw={persistence_rmse:.6f} accepted={profile['validation']['accepted']}"
    )


if __name__ == "__main__":
    main()
