# Vehicle-cabin calibration

`calibrate.py`, `calibrate_vehicle_fire.py`, and `validate_human_heat_stress.py` are dependency-free, deterministic pipelines for the aggregate thermal response, measured combustion source, and directional thermoregulation evidence used by `cockpit-world::digital_twin`.

## Source and license

- Dataset: *Experimental data set on the interior temperature of a vehicle (sedan): Measurements in hot-humid climate conditions in winter and spring*.
- Authors: Cesar Ramirez-Dolores, Jorge Andaverde, Juan Carlos Zamora-Luria, Lizeth Alejandra Lugo Ramírez.
- DOI/version: `10.17632/8mfgd8w9rg.1`.
- License: CC BY 4.0.
- Publisher file: `source/thermal-cabin-database.xlsx`.
- Required SHA-256: `9075e138317faa93be66891af8173dc9070e3782105d2f40f9f6f2273e89e777`.

The script refuses to process a workbook with another hash. It extracts Experiment d (1,302 measurements from a closed parked sedan over five continuous hours), writes the normalized CSV, fits a first-order thermal RC response on the first 70%, and evaluates recursive simulation on the remaining 30%.

## Reproduce

```bash
python3 calibration/calibrate.py
```

Expected evidence:

```text
source_sha256=9075e138317faa93be66891af8173dc9070e3782105d2f40f9f6f2273e89e777 observations=1302 holdout_rmse_c=2.026942 persistence_rmse_c=2.916170 accepted=True
```

The immutable output is `profiles/mendeley-sedan-v1.json`. Runtime coefficients in `DigitalTwinParameters::default()` must match this profile exactly.

## Claims boundary

The closed-cabin aggregate thermal response and the full-scale ICE-vehicle HRR/aggregate soot/CO yield source prior are experimentally calibrated in separate hash-gated profiles. CFK-derived AL2 COHb exposure/recovery is externally field-validated. Human heat-stress direction, resting stability, and humidity coupling are independently checked by `human-heat-stress-validation-v1`, but its sweat/evaporation parameters are not cohort-fitted. Inter-zone exchange, humidity mass balance, exterior-fire-to-cabin transfer, inter-zone smoke transport and fire-soot applicability, pressure equalization, individualized physiology outside the AL2 cohort and two-node thermal parameters remain in lower claim levels until matching experiments are added. This distinction is intentional: the repository does not label an engineering equation or cross-domain transfer boundary as vehicle-calibrated without source measurements and holdout evidence.

## Empirical validation baselines

The generated profile also records non-fitted empirical baselines, separately from the thermal calibration:

- **Smoke optics:** Mulholland and Croarkin's NIST flame-smoke measurements report `8.7 ± 1.1 m²/g` (95% expanded uncertainty). Runtime uses the unit-equivalent `0.0087 m²/mg`; this anchors Beer–Lambert extinction only.
- **Smoke deposition:** Ott, Klepeis and Switzer, DOI `10.1038/sj.jes.7500601`, made more than 100 ACH measurements across four vehicles and fitted 14 in-vehicle smoke-decay experiments. Their `k = 1.3a` deposition relation (`R²=0.82`) now replaces the fixed `900 s` loss rule and combines with ventilation in the conserved mass balance. The source aerosol was cigarette PM2.5, so transfer to vehicle-fire soot remains an explicit applicability boundary.
- **Parked infiltration:** Knibbs, de Dear and Atkinson, DOI `10.1111/j.1600-0668.2009.00593.x`, report approximately `0–1.4 ACH` for six stationary vehicles (more than 200 total measurements across stationary/moving and ventilation configurations). Runtime `0.25 ACH` is constrained to that parked-vehicle envelope.
- **Cabin absolute pressure:** Teleszewski and Gładyszewska-Fiedoruk, DOI `10.3390/s26020469` (CC BY 4.0), report 15 land-vehicle measurement series and the mean fit `p = 1013.6 − 0.112h` hPa over altitudes up to 1500 m, with `1.1 hPa` probe uncertainty. Runtime uses this fit in-domain. The paper explicitly did not analyze pressure-change rate, so HVAC/passive equalization dynamics remain unfitted.
- **CO uptake and recovery:** Alter, Dayan and Fleminger, DOI `10.3390/toxics14060488` (CC BY 4.0), validated CFK and MIL-STD-1472H against continuous CO monitoring and serial blood COHb from 100 young male armored-vehicle crew members. Runtime now uses the published integrated AL=2 equation (`A=241 min`, `B=1421 1/mmHg`, affinity `218`), which achieved peak-COHb RMSE `1.94%` and `r=0.61`. The model reproduces the published AL=2 `171 min` recovery half-time. Generalization outside that cohort and activity level remains limited.
- **Thermal physiology:** `validate_human_heat_stress.py` emits `human-heat-stress-validation-v1`. Che Muhamed et al., DOI `10.1080/23328940.2016.1182669` (CC BY-NC), measured core, mean-skin and mean-body temperature over 60 minutes at 31 °C, 70% VO₂max and 23–71% RH. Figure 1 is represented by approximate 60-minute means with ±0.10 °C digitization uncertainty and its SHA-256; the source image is not redistributed. Malcolm et al., DOI `10.3389/fphys.2018.00585` (CC BY 4.0), provide a closer vehicle-use boundary: 41 males completed randomized 60-minute seated trials at `39.6 °C/50.8% RH` and `21.2 °C/41.9% RH`, with higher hot-trial core/skin temperatures and `0.56 ± 0.38 L/h` sweat rate. Runtime now includes humidity-limited Lewis-relation evaporation and explicit core/skin sweat feedback. Acceptance covers resting stability, hot-vs-moderate direction, and RH ordering only; no exercise or passive cohort parameter fit is claimed.
- **Whole-vehicle combustion source:** NIST Fire Calorimetry Database DOI `10.18434/mds2-2314`, Vehicle2, provides the hash-gated 6,468-row source CSV. `calibrate_vehicle_fire.py` emits a 618-anchor, 10-second measured HRR curve plus published `0.0569 kg/kg` soot and `0.0590 kg/kg` CO yields. Non-anchor holdout RMSE must remain below 60 kW and beat persistence. Runtime derives heat, smoke mass and CO mass from that single measured source. The effective exterior-fire-to-cabin transfer fraction remains explicitly uncalibrated.

Reproduce all independent evidence profiles:

```bash
python3 calibration/calibrate.py
python3 calibration/calibrate_vehicle_fire.py
python3 calibration/validate_human_heat_stress.py
python3 calibration/verify.py
```

`scope.calibrated`, `scope.externallyValidatedModels`, `scope.experimentallyAnchoredNotFitted`, and `scope.physicsBasedNotDatasetCalibrated` are intentionally disjoint claim levels. A release must not promote lower tiers to calibrated without compatible raw observations, a fit split, and holdout evidence.
