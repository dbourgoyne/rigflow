# Rigflow Release Notes

---

## v0.1.2 — Client audio robustness

The client no longer crashes at startup when the default audio output device can't be opened.
Previously it aborted with an ALSA `snd_pcm_open` error — e.g. on a Raspberry Pi whose default
output is an unconnected HDMI sink (`Unknown errno (524)` / ENOTSUPP). It now tries the default
device, then any other output device, and uses the first that opens; if none can be opened it runs
**without local speaker audio** rather than aborting (radio control and the digital/FT8 paths are
unaffected, since those don't use the local speaker).

Cumulative: these binaries also include the **v0.1.1** Linux glibc packaging fix (they run on
Debian 12 "Bookworm" / Raspberry Pi OS).

---

## v0.1.1 — Packaging fix (Linux glibc)

A packaging-only release — **no functional changes** since v0.1.0.

The prebuilt **Linux** binaries (x86-64 and ARM64) are now built against an older glibc baseline
(Ubuntu 22.04 / glibc 2.35) so they run on **Debian 12 "Bookworm" and Raspberry Pi OS** (glibc 2.36)
— the primary server target. The v0.1.0 Linux binaries were built on Ubuntu 24.04 (glibc 2.39) and
failed at launch with `version 'GLIBC_2.39' not found` on Bookworm. (glibc is backward- but not
forward-compatible: a binary built against an older glibc runs on newer systems, not the reverse.)

Building from source was never affected, and the **macOS** binary is unchanged.

---

## v0.1.0 — Initial release

The first public release of Rigflow — a client/server SDR application for amateur radio that can
receive and transmit on HF over the network. Highlights:

**Receive**
- Modes: WFM, NFM, AM, USB, LSB, CW (CWU/CWL), and Data (USB for FT8/digital).
- Real-time spectrum and waterfall, with click, scroll, and keyboard tuning (mode-aware step sizes).
- Noise reduction (NR2), AGC, and squelch; bookmarks; RX IQ recording and playback.
- On-spectrum overlays of amateur **band** and **license-privilege** segments (ARRL/US),
  with hover details and a per-operator license selection.

**Transmit** (Hermes Lite 2)
- SSB from the microphone (USB/LSB) with a soft limiter and speech compressor.
- CW via straight key (Space bar) or Text-to-CW with F1–F4 memory macros, sidetone, and semi
  break-in; plus on-screen CW decode of received signals.
- Digital (FT8 / WSJT-X): selecting the Data mode auto-routes audio and automatically uses a
  clean, linear path (speech processing and receive AGC are bypassed); an in-app setup window
  shows the device names and CAT/PTT settings.
- Built-in two-tone and Spot/SWR transmit test aids.
- The N2ADR HF filter board is **enabled by default** for automatic per-band TX filtering
  (cleaner harmonics); it is a harmless no-op if the board isn't installed.

**Station**
- Remote operation over the network (lightweight WebSocket control + UDP media); multiple radios
  from one client.
- Per-operator and per-radio settings — each radio resumes exactly where its operator left it.
- Optional Hardrock-50 amplifier control (band tracking, ATU, SWR/power) over USB serial.
- A **Latency / Audio** panel reports live receive/transmit buffering and a measured network
  one-way delay — useful for remote / over-network operation.

**Hardware & platforms**
- Radios: Hermes Lite 2 (RX + TX), RTL-SDR (receive, incl. direct-sampling HF), plus WAV IQ playback
  and a built-in test tone. Optional Hardrock-50 amplifier.
- Server: Linux (x86-64 and Raspberry Pi / ARM). Client: Linux (x86-64, ARM64) and **macOS (Apple
  Silicon)**. Prebuilt binaries are provided for all of these; the macOS client is **unsigned** (a
  one-time Gatekeeper step is in the installation guide). *(Digital/FT8: on Linux via PipeWire/PulseAudio
  virtual audio **or** TCI; on macOS via TCI only. TCI is **experimental**; see below.)*

See the [Operator guide](operator-guide.md) for how to use these, the
[Signal path & expected behavior](signal-path.md) page for what Rigflow does to your audio
(and which behaviors are by design), and the [Validation](validation.md) page for transmit
signal-quality results.

---

## Known Issues & Workarounds

### macOS FT8 / digital over TCI is experimental

On macOS, digital modes (FT8/WSJT-X) run over a built-in **TCI** server instead of
a virtual audio device — no BlackHole and no microphone permission required. This
path is **experimental**: it has been confirmed transmitting and decoding on the
air with mainline WSJT-X (Rig = TCI, `127.0.0.1:40001`, *Use TCI Audio*), but it
has less on-air mileage than the Linux PipeWire path and has not been exercised
across the full range of TCI-capable apps (JTDX, MSHV) or band/rate combinations.
If you hit trouble, see the [Operator guide](operator-guide.md). The Linux digital
path is unaffected.

### Signal-strength, TX power, and SWR readings are approximate

The S-meter / dBm, TX forward/reverse power (watts), and SWR are **approximate
indications**, not lab-calibrated measurements. They use sensible default scaling
(like Quisk's defaults) and are good for *relative* judgements — comparing signals,
spotting a high-SWR condition, peaking a tune — but should not be treated as
absolute, instrument-grade values. No user calibration is required (or currently
offered); a per-rig calibration option may come in a future release.

### Receive CW sounds the same for CWU and CWL

Rigflow does not yet reject the opposite side of a received CW signal, so CWU and CWL
differ only on transmit, not on receive. This is a known limitation, not a bug; see
[Signal path & expected behavior](signal-path.md).

### What the Latency panel covers

The **Latency / Audio** panel measures Rigflow's own server↔client audio transport
(receive jitter buffer + network one-way; transmit mic ring + server queue). For
**FT8 / digital** — on both the PipeWire and TCI paths — the network and transmit
figures apply, but FT8 bypasses the jitter buffer, and the panel does **not** include
WSJT-X's own audio buffering or the sound-device buffers. Treat the totals as a close
estimate; see [Signal path & expected behavior](signal-path.md).

### HL2 not discovered after a simultaneous cold boot (direct Pi↔HL2 link)

**Applies to:** a Raspberry Pi server connected directly to the Hermes Lite 2 over
Ethernet, where the Pi hands the HL2 its IP via a DHCP server (e.g. `dnsmasq` on
`eth0`), and the Pi and HL2 are powered on **at the same time** (e.g. both fed
from one 13.8 V supply).

**Symptom:** After power-on you start `rigflow-server` and the HL2 is not listed.
A manual **Rescan** from the client also fails. The server log shows discovery
broadcasting correctly on the wired interface but finding nothing, e.g.:

```
HL2 discovery: broadcast sent on eth0 (192.168.1.1 → 192.168.1.255)
HL2 discovery: no devices found on LAN
```

You have to **power-cycle the HL2** (with the Pi already fully up) and then it is
found immediately — the same Rescan that just failed now succeeds.

**Root cause — not a Rigflow bug.** The Pi's `eth0` is up and Rigflow is
broadcasting on the correct subnet the whole time; the HL2 is simply not on the
network yet. The HL2 boots much faster than the Pi, so it sends its DHCP request
**before the Pi's DHCP server is running**. The request fails, the HL2 falls back
to a link-local / off-subnet address (e.g. `169.254.x.x`), and its firmware does
**not** keep retrying DHCP. Stranded on the wrong subnet, it never receives or
answers the subnet-directed discovery broadcast. Power-cycling the HL2 after the
Pi (and its DHCP server) are up lets it obtain its normal lease, land on the
right subnet, and respond. A faster/longer retry inside Rigflow cannot help —
discovery and Rescan run identical code, and the device is genuinely unreachable
until it re-acquires an address.

**Workarounds (any one resolves it):**

1. **Power-cycle the HL2 after the Pi is fully booted, then Rescan** from the
   client. This is the simplest manual fix and matches the diagnosis above — no
   server restart is needed, just a Rescan once the HL2 is back on-subnet.
2. **Give the HL2 a static IP** on the wired subnet (e.g. `192.168.1.10`, outside
   the DHCP pool), set in the HL2's own configuration. It is then on the correct
   subnet the instant it powers up, regardless of boot order, and is discovered
   at server startup with no power-cycle. *(Recommended permanent fix.)*
3. **Power the HL2 a few seconds after the Pi** (e.g. a power-on delay relay on
   the HL2's supply) so it requests DHCP after the Pi's DHCP server is running.

**Notes:**
- A DHCP *reservation* for the HL2's MAC does **not** fix this — the problem is
  the HL2 requesting before the DHCP server exists and not retrying, not which
  address it receives.
- An Ethernet **switch** between the Pi and HL2 keeps each link up independently
  of boot order and can help if a PHY link-negotiation race is also involved, but
  it does not address the DHCP-timing cause above.
