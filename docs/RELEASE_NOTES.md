# Rigflow Release Notes

---

## v0.1.0 — Initial release

The first public release of Rigflow — a client/server SDR application for amateur radio that can
receive and transmit on HF over the network. Highlights:

**Receive**
- Modes: WFM, NFM, AM, USB, LSB, CW (CWU/CWL), and Data (USB for FT8/digital).
- Real-time spectrum and waterfall, with click, scroll, and keyboard tuning (mode-aware step sizes).
- Noise reduction (NR2), AGC, and squelch; bookmarks; RX IQ recording and playback.

**Transmit** (Hermes Lite 2)
- SSB from the microphone (USB/LSB) with a soft limiter and speech compressor.
- CW via straight key (Space bar) or Text-to-CW with F1–F4 memory macros, sidetone, and semi
  break-in; plus on-screen CW decode of received signals.
- Digital (FT8 / WSJT-X): selecting the Data mode auto-routes audio, and an in-app setup window
  shows the device names and CAT/PTT settings.
- Built-in two-tone and Spot/SWR transmit test aids.

**Station**
- Remote operation over the network (lightweight WebSocket control + UDP media); multiple radios
  from one client.
- Per-operator and per-radio settings — each radio resumes exactly where its operator left it.
- Optional Hardrock-50 amplifier control (band tracking, ATU, SWR/power) over USB serial.

**Hardware & platforms**
- Radios: Hermes Lite 2 (RX + TX), RTL-SDR (receive, incl. direct-sampling HF), plus WAV IQ playback
  and a built-in test tone. Optional Hardrock-50 amplifier.
- Server: Linux (x86-64 and Raspberry Pi / ARM). Client: Linux and macOS. *(Digital/FT8 is supported
  on the Linux client only.)*

See the [Operator guide](operator-guide.md) for how to use these, and the
[Validation](validation.md) page for transmit signal-quality results.

---

## Known Issues & Workarounds

### Signal-strength, TX power, and SWR readings are approximate

The S-meter / dBm, TX forward/reverse power (watts), and SWR are **approximate
indications**, not lab-calibrated measurements. They use sensible default scaling
(like Quisk's defaults) and are good for *relative* judgements — comparing signals,
spotting a high-SWR condition, peaking a tune — but should not be treated as
absolute, instrument-grade values. No user calibration is required (or currently
offered); a per-rig calibration option may come in a future release.

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
