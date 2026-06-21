# Rigflow

**Rigflow** is a client/server software-defined-radio (SDR) application for amateur radio,
written in Rust. It can **receive and transmit on HF**, runs the radio over the network so the
operator can sit anywhere, and supports multiple radios from one client.

A lightweight **server** owns the radio hardware and DSP; a desktop **client** provides the
spectrum/waterfall, tuning, and controls. They talk over a small WebSocket control channel plus
UDP media, so the server can run on a low-power box at the antenna (e.g. a Raspberry Pi) while you
operate from your laptop.

> ⚠️ Rigflow controls transmitters and amplifiers. You are responsible for the legal and safe
> operation of your station — please read the **[Disclaimer](DISCLAIMER.md)** before transmitting.

---

## What it does

**Receive**
- Modes: WFM, NFM, AM, USB, LSB, CW (CWU/CWL), and **Data** (USB for FT8/digital)
- Real-time spectrum + waterfall, click/scroll/keyboard tuning, bookmarks
- Noise reduction (NR2), AGC, squelch; per-operator settings and presets
- RX IQ recording and playback

**Transmit** (Hermes Lite 2)
- **SSB** from your microphone (USB/LSB), with a soft limiter + speech compressor
- **CW** via straight-key (Space bar), or Text-to-CW with F1–F4 memory macros and sidetone
- **Digital (FT8 / WSJT-X)** — on **Linux** via virtual audio (PipeWire/Pulse) **or** TCI; on
  **macOS** via TCI only (experimental). An in-app setup window shows the exact settings
- Built-in two-tone and tune/SWR test aids

**Station**
- Remote operation over the network; multiple radios, one client
- Optional **Hardrock-50** amplifier control (band tracking, ATU, SWR/power) over USB serial

## Supported hardware

| Role | Hardware |
|---|---|
| HF transceiver (RX + TX) | **Hermes Lite 2** (primary) |
| Receive-only SDR | **RTL-SDR** (incl. direct-sampling HF) |
| Amplifier (optional) | **Hardrock-50** |
| No hardware | WAV IQ playback + a built-in test tone |

All sources are auto-discovered at server startup; the client picks a radio from the list.

## Platforms

- **Server:** Linux (x86-64 and Raspberry Pi / ARM)
- **Client:** Linux and macOS

## Quick start

Get Rigflow from the **[Releases page](https://github.com/dbourgoyne/rigflow/releases)** — a prebuilt
binary needs no Rust toolchain, just the runtime libraries listed in the
**[Installation guide](docs/installation.md)** — or build from source. Then:

```bash
# Run the server (auto-discovers RTL-SDR, Hermes Lite 2, WAV recordings, and a test tone)
./rigflow-server

# Run the client (in another terminal, or on another machine)
./rigflow-client
```

*(From a source checkout: `cargo run --release -p rigflow-server` / `-p rigflow-client` — see the
Installation guide for the toolchain and build libraries.)*

In the client: enter the server's IP (defaults to `127.0.0.1` for a single-box setup), click
**Connect**, pick a radio, and tune. See the **[Operator guide](docs/operator-guide.md)** for
day-to-day operation.

The server and client are driven live from the UI; their command-line options are minimal
(`--help` on either lists them — server: `--recordings-dir`, `--hr50-serial`; client:
`--window-size`).

## Documentation

- **[Installation guide](docs/installation.md)** — prerequisites, build, and connecting your radios
- **[Operator guide](docs/operator-guide.md)** — receiving, transmitting (SSB/CW), and digital (WSJT-X/FT8)
- **[Signal path & expected behavior](docs/signal-path.md)** — how Rigflow processes your audio, and what's by design
- **[Troubleshooting](docs/troubleshooting.md)** — common issues and fixes
- **[Validation](docs/validation.md)** — transmit signal-quality results
- **[Release notes](docs/RELEASE_NOTES.md)** — what's in this release and known issues
- **[Disclaimer](DISCLAIMER.md)** — operating responsibilities and warranty

## Reporting bugs & requesting features

Found a bug or have an idea? Please use
**[GitHub Issues](https://github.com/dbourgoyne/rigflow/issues/new/choose)** and pick **Bug
report** or **Feature request**. All reports go through GitHub Issues — there's no support email.
The bug form asks for your version, platform, radio, and logs, which makes problems much faster to
track down. For things that are already known, skim the
**[Release notes](docs/RELEASE_NOTES.md)** (known issues) and
**[Signal path & expected behavior](docs/signal-path.md)** first — a few reported "bugs" are
intentional.

## Status

Rigflow is **experimental** amateur-radio software for licensed operators and experimenters. It
works and is actively used on the air, but it may have rough edges. Verify your transmitted signal
and comply with your local regulations — see the [Disclaimer](DISCLAIMER.md).

## License

[MIT](LICENSE) © 2026 David Bourgoyne
