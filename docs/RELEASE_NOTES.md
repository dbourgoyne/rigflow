# Rigflow Release Notes

> Working document. Tracks user-facing changes, known issues, and workarounds as
> the project moves toward its first public release. Newest entries on top.

---

## Unreleased

### Changes

- **Each radio now remembers its own settings — resume exactly where you left
  off.** When you re-acquire a radio, its **Radio Control** (frequency, mode, filters,
  pitch, squelch, NR2, AGC, volume, CW sidetone/hang, CW decode, TX limiter/compressor),
  **Source Control**, and **Waterfall** settings are restored — and they're saved
  automatically whenever you change them.
  Settings are scoped per **operator *and* radio**, so two operators sharing one rig
  each keep their own setup, and switching between rigs no longer resets everything.
  (CW macros, mic device, bookmarks, license and server IP remain operator-wide, as
  before. A radio you've never used starts from your current settings.)

- **New "Data" (Data-USB) mode for FT8 and other digital programs.** Selecting
  **data** in the Demod row is USB on the air, but it carries its own filter setting
  (a wider 3 kHz default, remembered separately from voice USB), shows as **DATA-U**,
  reports the data mode (`PKTUSB`) over CAT, and **automatically enables RX audio
  routing** to WSJT-X/fldigi/etc. — entering Data turns routing on, leaving turns it
  off. So digital operating is now one click: pick **data**. As part of this, voice
  **USB now reports `USB`** over CAT (it previously always reported `PKTUSB`), and
  setting Data/Pkt in WSJT-X drives Rigflow straight into Data mode. The redundant
  "Digital Interface" panel under Advanced is gone (the WSJT-X / FT8 Setup window
  already shows the device names, and now shows a read-only RX-routing status too).

- **Radio Control layout tidy-up.** The **Audio** section (volume, CW sidetone) now
  sits at the top of Radio Control since it's used constantly, and the **Microphone**
  controls moved out of Audio into **Transmit** — so Audio is purely listening and
  Transmit holds how you transmit (mic for SSB, message/macros for CW). Section order
  is now Audio · Receive · Transmit.

- **New WSJT-X / FT8 setup helper.** A **"WSJT-X / FT8 Setup…"** button in Radio
  Control opens a window that shows the exact values to enter in WSJT-X — the audio
  **Input** (`RigflowDigitalRX`) and **Output** (`RigflowDigitalInput`) device names,
  and the CAT **Network Server** (`127.0.0.1:4532`, Rig = "Hamlib NET rigctl", PTT =
  CAT) — each with a **Copy** button and a live status indicator (green when the
  virtual audio device / rigctl port is available, red with the reason when not, e.g.
  PipeWire down or the port already in use). It also has the **RX Digital Output**
  decode-routing toggle, so you can finish setup without hunting through panels.

- **The default view is decluttered; bench/debug controls are behind a toggle.**
  Radio Control now shows only the everyday **Receive / Audio / Transmit** controls
  by default. A **"Show advanced & diagnostics controls"** checkbox (remembered per
  operator) reveals the two-tone test generator, TX-audio diagnostics, the speech
  limiter/compressor, and the digital interface when you want them. Also, the cryptic
  TX status **"TX FIFO idle"** is now the clearer **"TX underrun (recovered)"**.

- **The client now exits cleanly — it releases the radio and disconnects first.**
  Closing the window (the **[X]**) — or a terminal `kill` / Ctrl-C (SIGTERM /
  SIGINT) — no longer abruptly kills the process. The client first releases the
  radio (which un-keys the rig) and disconnects from the server, then quits. This
  means closing the app while transmitting leaves the rig safely un-keyed and frees
  the radio immediately instead of waiting for the server to time out the dropped
  connection. (A hard `kill -9` is still abrupt; the server's connection heartbeat
  and the radio's own TX watchdog remain the backstop there.)

- **First-run is no longer a dead end.** On a fresh install the client now opens
  the **Radio Operator** and **Rigflow Server** sections by default while you're
  disconnected (instead of leaving the only **Add Operator** and **Connect**
  buttons behind collapsed headers), shows a one-line cue pointing you at the two
  steps to get on the air, and seeds the server IP with **`127.0.0.1`** instead of
  a hardcoded LAN address — so a client and server on the same machine Connect with
  no typing, and a remote/Pi server is one edit away. Your per-operator server IP
  is still remembered, so this only affects the first run / newly created operators.

- **The server command line is simplified to three flags.** Everything else
  (frequency, mode, gain, ppm, sample rate, which source) is driven live by the
  client and discovery now finds every source automatically, so the old per-source
  tuning flags were redundant or dead and have been removed. What's left:
  `--help`/`-h`, `--recordings-dir PATH` (renamed from `--wav-dir`; the IQ-recording
  + WAV-playback directory), and `--hr50-serial auto|<path>[:baud]|none` — the amp
  baud is now folded into the value (e.g. `--hr50-serial /dev/ttyUSB0:19200`), so
  `--hr50-baud` is gone. Run the server with no arguments. As part of this, RTL-SDR
  device selection now opens the **acquired radio's own device index** (so a second
  RTL works via the radio list), rather than a single global `--rtl-device`.

- **The radio list now reflects real hardware (no phantom / stale entries).**
  RTL-SDR devices are **really enumerated** over USB — the server lists the
  actual dongles present (none when unplugged) instead of always advertising one
  phantom RTL that failed on acquire. A **Rescan** now also prunes a discovered
  radio that's genuinely gone *and* idle — a removed RTL, a deleted WAV, or a
  powered-off HL2 disappears from the list (a radio you're actively using is
  never removed). HL2 discovery was hardened to re-broadcast within each scan and
  dedupe replies, so a single dropped packet can't make a present HL2 flicker out.

- **One client per server, one server per host (enforced).** A server now serves a
  single client: a second client connecting to a busy server is cleanly rejected
  ("server already has a client") instead of causing undefined contention — so run
  one client per server (multiple clients on a host is fine, each to its own
  server). A second `rigflow-server` on the same host exits immediately with a
  clear "port already in use" message rather than half-starting. A WebSocket
  heartbeat detects dead/abandoned connections within ~40 s and frees the slot, so
  your own client's auto-reconnect after a network outage still works (it retries
  through the eviction window rather than being locked out).

- **HL2 link loss is now visible and survivable, and a shutdown can't leave the
  rig keyed.** A brief receive gap (Ethernet blip, switch hiccup) no longer tears
  the whole radio down: the server keeps the worker alive, surfaces the radio as
  **"not responding"** (for any SDR — an HL2 link drop or an RTL dongle pulled
  mid-stream) in the Source Status panel and the status light, and resumes RX
  in place when packets return. Only a *sustained* outage (~10 s) ends the
  session, after which the client's auto-reconnect re-acquires and
  re-initializes a power-cycled device. Separately, the server now handles
  **Ctrl-C / SIGTERM** by stopping all radios before exit, which **un-keys the
  HL2** — previously a shutdown mid-transmit left PTT asserted until the radio's
  own watchdog timed out. (A hard kill, `SIGKILL`, still relies on that
  watchdog.)

- **Corrupt config no longer leaves you stuck or silently wipes everything.** If
  an operator settings file or the app-state file is unparseable, the client now
  quarantines the bad file (renamed to `<name>.corrupt-<timestamp>`), resets just
  that file to defaults, and shows a Warning in the Status / Problems area —
  instead of an unusable operator (previously) or a silent reset of *all* settings
  (previously, on a corrupt app-state file). A file written by a **newer** Rigflow
  build (e.g. after a downgrade) is **left untouched on disk**: the session runs on
  defaults and you're told to upgrade to use it, so good settings aren't destroyed.

- **The client now auto-reconnects after a network drop.** A transient blip
  (WiFi hiccup, brief server restart) no longer ends the session: the control
  plane reconnects with exponential backoff (1→2→4→8→16→30 s cap) and
  **automatically re-acquires the radio you had**, restoring full state (the
  server resends a runtime snapshot — no settings are lost) and resuming audio.
  Details:
  - Only an *unexpected* drop reconnects; an explicit **Disconnect** does not.
  - If you reconnect faster than the server releases your old lease, re-acquire
    retries for ~35 s until the radio frees, then reports an error if it can't.
  - Status shows "reconnecting (attempt N)…" / "re-acquiring radio…" as a
    transient Warning in the Status / Problems area, escalating to an Error only
    if re-acquire ultimately gives up.
  - Known limitation (this release): while reconnecting there is no dedicated
    "stop reconnecting" button — the Server button reads "Connect" and
    re-triggers an immediate attempt. Auto-reconnecting the *last* radio across a
    full app restart is not yet implemented.

- **Subsystem failures are now shown on screen, not just in the log.** A
  fixed **Status console** docked at the bottom of the left panel (always visible,
  scrollable when the list is long) — plus an **LED-style status light** in the top
  status bar (green = all OK, amber = warnings only, red = an error; hover for the
  list) — surfaces active failures with their specific reason instead of failing
  silently. Covered:
  - **rigctl (CAT) bind failure** — e.g. port 4532 already in use (previously
    WSJT-X would just say "can't open rig" with no hint from Rigflow).
  - **Digital / PipeWire unavailability** — virtual audio device creation
    failures now show the underlying reason (e.g. `pactl` not found / PipeWire
    down) rather than a red dot buried in *Advanced*.
  - **Amplifier serial open failure** — an explicitly configured `--hr50-serial`
    path that can't be opened (wrong path, permission, baud) is surfaced; a bad
    `dialout` permission no longer fails silently. (Auto-detect finding no
    amplifier stays log-only, so stations without an HR50 see nothing.)
  - **Server connection failures** — connect/acquire/connection-drop errors are
    visible even though the Server panel is collapsed by default.

- **HR50 amplifier serial port is now auto-detected.** `--hr50-serial` defaults to
  `auto`: USB-serial ports are narrowed by USB VID/PID to known converter chips
  (FTDI, Microchip MCP2200, Silicon Labs CP210x, Prolific, CH340), then each is
  probed with a read-only `HRRX;` and only a port that answers as a Hardrock-50
  is used. The baud rate is auto-scanned (`--hr50-baud` is tried first). This
  removes the previous hard-coded `/dev/ttyUSB0` default, which could send CAT
  bytes to an unrelated serial device. An explicit path (e.g.
  `--hr50-serial /dev/ttyUSB0`) still opens that device directly; `none` disables
  amplifier polling.

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
