# Transmit signal validation

Rigflow generates the transmitted signal in software, so its spectral quality can be measured
directly and checked automatically. This page summarizes how the transmit path is validated and the
results on the developer's station. It is a summary, not a calibration certificate — **you are
responsible for verifying your own station** (see the [Disclaimer](../DISCLAIMER.md)).

## How it's validated

- **Automated software tests** measure the properties that are Rigflow's own — sideband and carrier
  suppression, intermodulation, CW keying, and digital splatter — by analyzing the IQ the modulator
  produces. They run as part of the test suite (`cargo test -p rigflow-server`), so a regression that
  dirtied the signal would fail the build.
- **On-air measurements** confirm the things the hardware owns (harmonics, power) on the real radio
  with a spectrum analyzer.

## Software-measured (the modulator / DSP)

| Property | Result |
|---|---|
| Opposite-sideband (image) suppression | **≈ 74 dB** (USB and LSB) |
| Carrier suppression | **≈ 99 dB** |
| Two-tone IMD added by the modulator | **none measurable** (numerical floor) |
| Two-tone IMD, full TX processing at normal level | **≈ −85 dBc** |
| CW key-click sidebands (8 ms raised-cosine keying) | **≈ −73 dBc** |
| Digital (FT8-style) out-of-band splatter | **≈ −69 dBc** |

These are *ratios* measured by the same analysis, so they're independent of absolute level
calibration.

## On-air measured (Hermes Lite 2)

- **Harmonics (2nd–5th), all HF bands 160–10 m:** every harmonic is **≥ 57 dB below the carrier**
  (worst case −57 dBc on 12 m; most ≥ 60 dBc). That is well inside the FCC §97.307 spurious-emission
  limit (−43 dBc for HF) — **≥ 14 dB of margin.**
- **Output power:** ~3.5–5 W across the bands (near the HL2's rated output).

## What this means

As measured here, Rigflow's transmitted SSB, CW, and digital signals are clean and within amateur
spectral-purity limits. The opposite-sideband and carrier suppression are excellent, intermodulation
is low, CW keying is click-free, and harmonics pass with comfortable margin.

## Caveats

- **Absolute power and SWR readings in the UI are approximate** (relative indicators, not
  lab-calibrated). Use instrument-grade gear for absolute measurements.
- These results are from one station (a Hermes Lite 2 at ~13.8 V). **Your results will vary** with
  your radio, supply voltage, drive level, and configuration.
- Spectral purity above the fundamental ultimately depends on your antenna system and any external
  amplifier. **Verify your own signal before operating** and comply with your local regulations.
