# Operator guide

Day-to-day operation of Rigflow. This assumes the server and client are installed and running — see
the **[Installation guide](installation.md)** if not.

> Rigflow transmits. Read the **[Disclaimer](../DISCLAIMER.md)**, operate within your license, and
> verify your signal.

---

## Getting on the air

1. In the client's **Radio Operator** section, select (or add) your operator profile.
2. Enter the server IP (or leave `127.0.0.1` for a single machine) and click **Connect**.
3. In **Radios**, pick a radio to **acquire** it. The controls and the spectrum/waterfall come alive.
4. Tune, choose a mode, and operate. Click **Release** (or just close the client) when done.

Each radio remembers where you left it: re-acquiring restores your frequency, mode, filters, and
display for that radio and operator.

## Screen layout

- **Left panel** — all controls, grouped into collapsible sections (Radio Operator, Station, Server,
  Radios, **Radio Control**, **Source Control**, Waterfall, Bookmarks, **DX Cluster**).
- **Center** — a status bar (frequency, mode, S-meter, dBm, TX/RX, SWR) above the **spectrum** and
  **waterfall**.
- **Logbook windows** open on demand rather than living in the left panel: **`L`** opens the
  log-entry window and **`V`** toggles the **Contacts** view, whose toolbar holds import, export,
  online sync, and the callbook settings.

Advanced and diagnostic controls are hidden by default. Tick **"Show advanced & diagnostics
controls"** at the bottom of Radio Control (and Source Control) to reveal them.

---

## Tuning

You can tune several ways, and they all respect each radio's frequency limits:

- **Click** anywhere on the spectrum or waterfall to jump there (snapped to the Snap grid).
- **Mouse wheel** over the spectrum/waterfall — steps the **dial** by the **Snap** value; **Ctrl+wheel zooms** the display.
- **Arrow keys** — **← / →** step the **dial** by the Snap value (same as the wheel); **↑ / ↓** move the **LO** (the center of the display) in larger, mode-appropriate steps. Arrow keys are ignored while you're typing in a text field.
- **`C`** (cursor over the spectrum/waterfall) — re-center the display on the current signal.
- The **LO dial / LO offset** widgets above the spectrum — scroll a digit to set it directly.

### Tuning step ("Snap")

The **Snap** dropdown next to the LO dial sets the base **dial** step — the amount every mouse-wheel notch and **← / →** press moves. Pick from **1 Hz … 10 kHz**. Each mode remembers its own Snap value (defaults: SSB 1 kHz, CW 50 Hz, AM/NFM 5 kHz, Digital 1 Hz), saved per operator; under dual-watch VFO B keeps its own Snap.

The wheel and **← / →** use the same relative model, scaled by the modifier keys:

| Modifier | Step |
|---|---|
| (none) | **×1** the Snap value |
| **Shift** | **×10** the Snap value (accelerate) |
| **Alt** | **×0.1** the Snap value (decelerate) |

The step never drops below **1 Hz** (so Alt on a small Snap won't attempt fractional-Hz tuning). Example: with Snap = 1 kHz, the wheel and ←/→ move 1 kHz per step, **Shift** → 10 kHz, **Alt** → 100 Hz.

**↑ / ↓ are different:** they move the whole display window (the LO) in coarse, mode-appropriate steps so you can sweep across a band quickly — **1 kHz** on CW/SSB/Data (25 kHz with **Shift**), 10 kHz on AM, 25 kHz on NFM, 200 kHz on WFM. These are independent of the Snap value.

### Bands

There's a **Band** row in **Source Control** (Hermes Lite 2): click a band to jump straight to its
default frequency and mode. The highlighted band is *derived from your current frequency*, so it always
shows where you're tuned — however you got there (band button, click, wheel, keyboard, or a bookmark).
The HL2's transmit **low-pass filter follows the band automatically**; there's nothing to switch by hand.

Tuning **within** a band just moves the dial under the same display; jumping to a **different** band
recenters the waterfall on the new frequency. **[Bookmarks](#bookmarks)** are the quickest way to hop
to specific frequencies.

---

## Receiving

In **Radio Control → Receive**, pick a **Demod** mode and shape the audio:

- **Modes:** WFM · NFM · AM · LSB · USB · **DATA** (digital/FT8) · CWU · CWL.
- **Filter bandwidth** and, for CW/Data, **pitch** — each mode remembers its own setting.
- **Squelch**, **NR2** (noise reduction), and **AGC** (strength 0 = off-equivalent, up to full).
- **CW decode** (CW modes) — decodes received CW to on-screen text.

**Audio** (top of Radio Control) holds the receive **Volume**.

The **waterfall/spectrum** display is configured in the **Waterfall** section — zoom and either
adaptive or manual normalization (top/range in dB).

---

## Dual-watch, split & RIT/XIT (Hermes Lite 2)

On hardware that supports it, Rigflow can run **two receivers at once** and transmit **split**. These
controls appear in the **VFO** section (and inline on the status bar); they're hidden on receive-only
sources.

- **Dual-watch — two receivers.** Enable a second receiver (**VFO B**) alongside VFO A. The center
  pane splits into stacked spectrum + waterfall panes, and you hear **A in the left ear, B in the
  right**. Each VFO has its own frequency, mode, and full receive processing (filter, pitch, squelch,
  NR2, noise blanker, auto-notch, AGC, deemphasis). An **Active VFO: A | B** selector points the
  Receive controls at the receiver you're adjusting; both volumes stay visible so you can mix the two
  live.
- **Split.** Transmit on the selected TX VFO while listening on the other — the classic DX-split
  setup. The transmitting VFO is marked **▶TX** on the status bar.
- **RIT / XIT.** A per-VFO **RIT** offset nudges your *receive* frequency without moving your transmit
  frequency (chasing a drifting station); **XIT** offsets your *transmit* frequency the same way.
- **Hotkeys:** **`X`** swaps TX focus between A and B, **`=`** copies the active VFO to the other
  (A=B), and **`B`** bookmarks the current frequency.

Each VFO keeps its own tuning **Snap** step (see above), and the per-band memory restores frequency +
mode as you move around.

---

## Transmitting — SSB (voice)

1. Set the mode to **USB** or **LSB**.
2. In **Radio Control → Transmit**, choose your **Microphone** and set **Mic Gain**; watch the level
   meter and keep the **clip** indicator off on voice peaks.
3. **Hold the Space bar to transmit** (push-to-talk); release to receive.

Optional **TX processing** (under **Advanced**): a soft **limiter** (peak protection) and a **speech
compressor** (more average talk power). Leave the limiter on; add compression to taste.

### Voice keyer

To save your voice on a long CQ or a pileup, record a clip once and let Rigflow send it. In the
**Transmit** section you can **record** a short message from your microphone, **preview** it (played
locally, off-air), **delete** it, and **transmit** it — Rigflow keys the radio and plays the clip
through the normal SSB transmit path. Clips are stored **per operator**, so each operator keeps their
own CQ. It's ordinary voice transmit under the hood, so the same focus/PTT safety rules below apply.

> The Space bar (SSB and CW) only keys when no text field has focus (so it doesn't fight typing), and
> transmit **stops if the client window loses focus** — switching to another window always drops you
> back to receive, so to keep transmitting keep the client focused. Always confirm you're transmitting
> into an antenna or load before keying.

## Transmitting — CW

1. Set the mode to **CWU** or **CWL** and set the **pitch** (Receive).
2. **Straight key:** hold **Space** to key down. The server provides semi-break-in with a hang time.
3. **Text-to-CW:** in **Transmit**, type a message, set the **speed (WPM)**, and **Send** — or use the
   **F1–F4 memory macros** (edit their text in the macro fields). **Sidetone volume** and **Hang
   time** are in Transmit too.

The client generates a local sidetone and the keying envelope is shaped to avoid key clicks.

## Transmit test aids

With **"Show advanced & diagnostics controls"** ticked:

- **Two-Tone Test** (Radio Control → Diagnostics, USB/LSB) — a clean two-tone for checking linearity.
- **Spot / SWR** and **SWR Sweep** (Source Control → Diagnostics, HL2) — a short low-power carrier to
  read SWR / peak an antenna tuner. **TX Drive** and **Spot Level** live in Source Control.

---

## Digital modes (WSJT-X / FT8)

There are two transports, and which you use depends on your platform:

- **Linux** — use **either** the **virtual-audio** method (PipeWire/PulseAudio; the default, and works
  with any digital app including FLDigi and JS8Call) **or** the **TCI** method (experimental;
  for TCI-capable apps). Both are described below.
- **macOS** — **TCI only** (experimental). macOS has no virtual audio device, so the PipeWire method
  does not apply.

> **Clean signal path is automatic in DATA mode.** WSJT-X owns the modem, and FT8 is a single
> constant-envelope tone that needs a flat, linear path. In **DATA** mode Rigflow therefore **bypasses
> the TX speech compressor and limiter** (their make-up gain and clipping would only add IMD/splatter)
> and **disables receive AGC** (a pumping AGC corrupts the relative signal levels the FT8 decoder relies
> on). This happens regardless of your SSB-voice settings — you don't need to turn anything off by hand.
> Set transmit level with **TX drive** so the tone sits in the linear region. (CW is unaffected: it uses
> a separate enveloped transmit path that never runs the compressor/limiter.)

### Linux — virtual audio (PipeWire/PulseAudio)

Rigflow makes digital nearly one-click:

1. Set the mode to **DATA**. This is USB on the air with a wider default filter, and it **automatically
   routes receive audio** to the digital virtual sound device — no manual audio plumbing.
2. Open the **WSJT-X / FT8 Setup** window (Radio Control → **Advanced** → *WSJT-X / FT8 Setup…*, with
   advanced controls shown). It lists exactly what to enter in WSJT-X:
   - **Soundcard Input:** `RigflowDigitalRX`  ·  **Output:** `RigflowDigitalInput`
   - **CAT (Radio):** Hamlib **NET rigctl**, host/port **`127.0.0.1:4532`**
   - **PTT:** **CAT** (transmit is keyed over the rig-control link)
3. In WSJT-X, set Mode = FT8, pick those devices and the CAT settings, and operate. Selecting a WSJT-X
   data/pkt mode also drives Rigflow into Data mode automatically.

Leaving **DATA** turns RX routing back off.

On Linux you can instead use the **TCI** method below (the same one macOS uses) for TCI-capable apps —
the Setup window lists it as "Method 2".

### macOS — TCI (experimental)

macOS has no virtual audio device, so digital uses **TCI** instead: WSJT-X carries CAT, PTT, and **both
audio directions over one localhost connection** — no BlackHole and no microphone permission. The client
runs a TCI server at `ws://127.0.0.1:40001` whenever it's running.

In WSJT-X:

- **Settings → Radio:** Rig = **TCI**, TCI Server = **`127.0.0.1:40001`**, and tick **Use TCI Audio**.
- **Settings → Audio:** Input and Output both = the **TCI** device.
- **Mode:** Data/Pkt (or USB). Set Rigflow to **DATA**.

Then operate normally — no soundcard or CAT plumbing to configure. This path also works on Linux, but the
PipeWire route above is the default there.

### WSJT-X: Split Operation

Set WSJT-X's **Settings → Radio → Split Operation** to **Fake It** (recommended) or **None** — **not
"Rig"**. Rigflow has a single VFO and doesn't implement rig split, so "Rig" leaves WSJT-X trying to set a
split it can't, and it stalls or reports a frequency mismatch. **Fake It** keeps the transmit tone in
WSJT-X's preferred range by nudging the dial on transmit, and Rigflow handles that as ordinary in-band
tuning. This applies to both the virtual-audio (rigctld) and TCI paths.

---

## Logging contacts

Rigflow has a built-in contact log (an electronic logbook). Contacts are stored per operator in a
local database, and everything exports to standard **ADIF** for LoTW, eQSL, QRZ, or any other logger.

### Set your station first

Before logging, fill in the **Station** panel: your **callsign**, **grid**, and (for US awards)
**state / county / CQ & ITU zones**. These become the `MY_*` fields written into every contact you
log, so awards and confirmations match correctly. The callsign and operator name are per operator;
the physical location (grid/state/county/zones) is shared across operators on the same rig.

> The station snapshot is copied into each contact **at the moment you log it** — so if you correct
> your station details later, only *future* contacts pick up the change. See
> **[Troubleshooting](troubleshooting.md)** to fix already-logged contacts.

### Logging a contact

1. Press **`L`** to open the log-entry window. It captures the current **frequency and mode**; the
   **time is stamped when you press Log**, not when the window opens — so you can pop it open early in
   a pileup, prep the call, and the logged time still reflects when you actually made the contact.
2. Type the **callsign** and fill the exchange (**RST sent/received**, **name**, **grid**, comment).
   With a callbook configured (below), **name and grid auto-fill** as you type the call.
3. **Worked-before hints** tell you at a glance whether the callsign is new, new on this band, or a
   dupe.
4. Press **Log** (Enter) to save.

**WSJT-X / FT8 contacts log themselves** — when WSJT-X logs a QSO, Rigflow ingests it automatically
into the active operator's log (skipping duplicates), so digital contacts need no manual entry.

### The Contacts view

Press **`V`** to open the **Contacts** view and browse your log. You can **filter** (by band, mode, date, callsign,
and more), see **confirmation badges** (which contacts LoTW / eQSL / QRZ have confirmed), **edit** any
field of a contact (including date/time — the place to correct a late or mis-stamped entry),
**delete** contacts, and **select multiple** rows for a bulk action. The filter you set here is
**shared with export** — what you see is exactly what gets written.

### Import & export (ADIF)

From the **Contacts** view's toolbar (**Import… / Export…**):

- **Export** writes ADIF for the current filter. It's a *plan → write* flow, streams large logs to
  disk without stalling the UI, and can do an **incremental** "since last export" so you only send new
  contacts to an external logger. (The file picker needs a desktop **file-chooser portal**; see
  Troubleshooting if **Browse…** does nothing.)
- **Import** reads an ADIF file with a **plan → preview → commit** flow: it shows what will be added
  before changing anything, skips near-duplicates, and imports **confirmations** — e.g. a LoTW
  `lotwreport.adi` marks your matching contacts confirmed rather than creating duplicates.

The log is kept in a local SQLite database (the source of truth) plus an **append-only ADIF journal**
that captures every contact as it's made. Export from the database for a current-state ADIF; the
journal is a historical capture record and is never rewritten.

### Online sync (LoTW / eQSL / QRZ)

From the **Contacts** view's toolbar (**Sync…**) you can sync directly with the online services:

- **LoTW** — download confirmations (marks matching contacts confirmed).
- **eQSL** — upload contacts and download confirmations.
- **QRZ Logbook** — upload contacts and download confirmations.

Enter each service's credentials once; they're stored in your operating system's **secure keyring**
(with an encrypted-file fallback if no keyring is available) and never written to logs or the ADIF.
Confirmed contacts show a coloured badge in the Contacts view.

> **US county format:** LoTW/tqsl expects `MY_CNTY` as `STATE,County` (e.g. `MD,Carroll`), not a bare
> county name. Set your county that way in the Station panel.

---

## Callbook lookup

As you type a callsign in the log-entry window, Rigflow can look it up online and **auto-fill the
name, grid, QTH (city), state/county, country, and DXCC/zones**. The "via …" note under the fields
shows where the data came from and a human-readable location, e.g. `via QRZ · Chicago, IL · United
States · DXCC 291`.

**Providers**, tried in your priority order (first match wins):

| Provider | Account | Notes |
|---|---|---|
| **QRZ XML** | qrz.com login + **XML Logbook Data** subscription | Full data. This is your qrz.com login, **not** the QRZ Logbook API key used for QSO sync. |
| **HamQTH** | free account | Username + password. |
| **Callook** | none | US callsigns only, no login. |

Underneath all of them is an always-on **offline prefix baseline** that fills the **DXCC entity and
CQ/ITU zones** from the callsign prefix with no network — so even without an account (or offline), a
contact still gets its country and zones, and WSJT-X captures get a DXCC.

**Configure** in the **Callbook** window (Contacts view → **Callbook…**): enable each provider, set the
priority order, and enter credentials — stored in the same secure keyring as the sync services.
**Anything you type always wins** over the callbook, and an online result overrides the offline
baseline field-by-field, so a lookup never clobbers your own edits.

---

## DX Cluster

Connect to a **DX-cluster** node to see where stations are being spotted — overlaid live on your
spectrum/waterfall and in a scrollable spot list — and click a spot to tune straight to it.

**Set it up** in the **DX Cluster** panel → **Configure…**: tick **Enabled**, pick a node from the
built-in list (**VE7CC** by default, plus NC7J / W3LPL / HRD) or enter a **custom host/port**, and
confirm your **callsign** (public nodes log in with just your call — no password). **Save & apply** to
connect; the panel shows the connection status.

**Working spots:**

- The **spot list (band map)** is the main view — every spot that passes your filter, sorted by
  frequency, each row **click-to-tune** (it recenters on the spot). This shows band-wide activity that
  the narrow waterfall span can't.
- **Markers** appear on the spectrum and waterfall for spots inside the currently displayed span —
  callsign + a fade-with-age tick, click to tune.
- A **"N received · M shown"** line tells you how many spots have arrived versus how many pass your
  filter.

**Filter** (in Configure) by **current band only** (default) and by **mode**, to cut the firehose down
to what you're working. Spots age out automatically, and the connection reconnects itself if the node
drops. Cluster settings are locked while you're connected to a rigflow **server**, like other operator
settings.

---

## Bookmarks

Save the current frequency/mode as a **bookmark** (Bookmarks section) and recall it later; you can
mark one as the default to auto-apply on acquire. Bookmarks are per-operator.

## Recording & playback

**Source Control → Recording** records the received IQ to a file (frequency embedded in the name).
Recordings appear back in the **Radios** list as playable "radios," so you can replay a band later.

## Operators, settings & persistence

- Settings are saved **per operator** and, for operating state, **per (operator + radio)** — so two
  operators sharing one rig each keep their own setup, and each radio resumes where its operator left
  it (frequency, mode, filters, volume, NR2/AGC, TX processing, waterfall).
- Library/hardware items (CW macros, mic device, bookmarks, license, server IP) are operator-wide.
- The **contact log**, **callbook** setup, **DX-cluster** setup, and **voice-keyer** clips are all
  per operator too; service/callbook **credentials** live in the OS secure keyring (file fallback).
- Operator settings are **locked while connected** to a server (to avoid surprise changes mid-session).

## Understanding the control sections

A consistent rule governs the collapsible sections:

- **Status** — read-only information (telemetry, meters).
- **Diagnostics** — system-testing tools that key/exercise the rig (Two-Tone, Spot/SWR, SWR Sweep,
  TX Test Tone). Not normal operation.
- **Advanced** — normal-operation controls you rarely change (TX limiter/compressor, WSJT-X setup).

Diagnostics and Advanced are hidden until you tick **"Show advanced & diagnostics controls."**

---

To understand *what* Rigflow does to your audio and which behaviors are intentional, see
**[Signal path & expected behavior](signal-path.md)**. If something doesn't work as
expected, see **[Troubleshooting](troubleshooting.md)**.
