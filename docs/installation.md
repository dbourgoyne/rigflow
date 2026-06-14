# Installation guide

This guide covers building Rigflow and connecting your radios. Rigflow is two programs:

- **`rigflow-server`** — owns the radio hardware and does the DSP. Run it on the machine the radios
  are plugged into (often a small Linux box or Raspberry Pi at the antenna).
- **`rigflow-client`** — the desktop UI. Run it on your operating computer (Linux or macOS).

They can be the **same machine** (server + client on one computer) or **two machines** on the same
network. Build from source on each machine that runs a component.

---

## 1. Prerequisites

### Rust toolchain (server + client)

Install Rust with [rustup](https://rustup.rs) (the client uses Rust **edition 2024**, so you need
**Rust 1.85 or newer**):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustc --version    # 1.85.0 or newer
```

### System libraries

**Server (Linux, including Raspberry Pi):** the build needs a C toolchain and **libusb** (used by the
RTL-SDR driver). The server does *not* need an audio library.

```bash
# Debian / Ubuntu / Raspberry Pi OS
sudo apt install build-essential pkg-config libusb-1.0-0-dev
```
*(Fedora: `gcc pkgconf-pkg-config libusb1-devel` · Arch: `base-devel libusb`)*

**Client (Linux):** the UI plays audio through ALSA, so it needs the ALSA dev headers:

```bash
sudo apt install build-essential pkg-config libasound2-dev
```

For **digital modes (WSJT-X/FT8)** the client also needs **PipeWire** (or PulseAudio), which is
standard on modern Linux desktops — no extra step on most systems.

**Client (macOS):** install the Xcode Command Line Tools (`xcode-select --install`). Audio uses
CoreAudio (built in). The macOS client supports **receive, SSB/CW transmit, and FT8/digital** —
the latter via a built-in **TCI** server (no BlackHole or virtual audio driver needed; see the
operator guide). FT8 over TCI is **experimental**.

---

## 2. Build

From the repository root, on each machine:

```bash
cargo build --release
```

The binaries land in `target/release/` (`rigflow-server`, `rigflow-client`). The first build takes a
while (especially on a Pi); later builds are fast.

---

## 3. Run

**Single machine** (server and client on one computer):

```bash
# terminal 1
cargo run --release -p rigflow-server

# terminal 2
cargo run --release -p rigflow-client
```

The client defaults its server address to `127.0.0.1`, so click **Connect** and you're talking to the
local server.

**Two machines:** run `rigflow-server` on the radio host, `rigflow-client` on your laptop, and enter
the **server's IP address** in the client before connecting (see [Networking](#5-networking)).

Both programs are driven live from the UI; their command-line options are minimal:

- Server: `--help`, `--recordings-dir PATH` (where WAV/IQ recordings live),
  `--hr50-serial auto|<path>[:baud]|none` (amplifier port — see below).
- Client: `--help`, `--window-size <WxH>` (default `1280x720`).

---

## 4. Connecting your radio(s)

All sources are **auto-discovered** when the server starts; the client lists them and you pick one.
There's nothing to configure for a basic setup — plug the hardware into the **server** host.

### Hermes Lite 2 — HF transceiver (primary, RX + TX)

Connect the HL2 by **Ethernet** to the same network/subnet as the server host (a direct cable to the
Pi also works). The server finds it automatically by network discovery — no IP to enter. Power it up
before (or shortly after) starting the server; if it appears late, the client's **Radios** list has a
**Rescan** button.

### RTL-SDR — receive-only

Plug the RTL-SDR into a **USB** port on the server host. Two one-time Linux setup steps:

1. **Free the device from the TV-tuner driver** (the kernel's DVB driver otherwise grabs it):
   ```bash
   echo 'blacklist dvb_usb_rtl28xxu' | sudo tee /etc/modprobe.d/blacklist-rtl.conf
   ```
   then reboot (or unplug/replug after `sudo modprobe -r dvb_usb_rtl28xxu`).
2. **Allow non-root USB access** — install the distro's `rtl-sdr` package for its udev rules
   (`sudo apt install rtl-sdr`), or add your own rule, then unplug/replug.

It then shows up in the radio list. (HF reception uses direct sampling — selectable in Source Control.)

### Hardrock-50 — amplifier (optional)

Connect the HR50's **USB serial** port to the server host. Add yourself to the `dialout` group so the
server can open the port, then log out and back in:

```bash
sudo usermod -aG dialout $USER
```

By default the server auto-detects the HR50 (`--hr50-serial auto`); pass an explicit
`--hr50-serial /dev/ttyUSB0:19200` to force a port/baud, or `none` to disable. The HR50's *radio-side*
menu must have its serial port enabled at a matching baud — see the Hardrock-50 manual. (If you use
the amp's ACC port rather than USB, the baud is set by a different menu item; see
[Troubleshooting](troubleshooting.md).)

### No hardware?

The server always offers a built-in **test tone** and any **WAV IQ recordings** in the recordings
directory (`--recordings-dir`), so you can try the client without a radio.

---

## 5. Networking

When server and client are on different machines, make sure these are reachable from the client to
the server host:

| Port | Protocol | Purpose |
|---|---|---|
| 9000 | TCP | WebSocket control |
| 9001 | UDP | Client registration |

The server then streams audio and waterfall data **back to the client over UDP**, so the client host
must accept inbound UDP from the server (home LANs typically do; a restrictive firewall may need a
rule). Put both machines on the same LAN/subnet for discovery and lowest latency.

---

## 6. First connection (verify it works)

1. Start `rigflow-server` on the radio host.
2. Start `rigflow-client`; under **Radio Operator**, add an operator (your callsign).
3. Enter the server IP (or leave `127.0.0.1` for one machine) and click **Connect**.
4. The **Radios** list populates. Pick the **test tone** (or a radio) to confirm audio and the
   waterfall are flowing.
5. Pick your HL2/RTL-SDR and tune.

Day-to-day operation — tuning, modes, transmitting, digital — is in the
**[Operator guide](operator-guide.md)**. If something doesn't connect or appear, see
**[Troubleshooting](troubleshooting.md)**.

> Before transmitting, read the **[Disclaimer](../DISCLAIMER.md)** and verify your signal.
