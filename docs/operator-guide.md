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

- **Left panel** — all controls, grouped into collapsible sections (Radio Operator, Server, Radios,
  **Radio Control**, **Source Control**, Waterfall, Bookmarks).
- **Center** — a status bar (frequency, mode, S-meter, dBm, TX/RX, SWR) above the **spectrum** and
  **waterfall**.

Advanced and diagnostic controls are hidden by default. Tick **"Show advanced & diagnostics
controls"** at the bottom of Radio Control (and Source Control) to reveal them.

---

## Tuning

You can tune several ways, and they all respect each radio's frequency limits:

- **Click** anywhere on the spectrum or waterfall to jump there.
- **Mouse wheel** over the spectrum/waterfall — steps the dial; **Ctrl+wheel zooms** the display.
- **Arrow keys** — **← / →** step the dial; **↑ / ↓** move the LO (the center of the display).
- **`C`** (cursor over the spectrum/waterfall) — re-center the display on the current signal.
- The **LO dial / LO offset** widgets above the spectrum — scroll a digit to set it directly.

The step size **adapts to the mode** (and modifier keys), so it's sensible on every band:

| Mode | Wheel / ←→ | with **Shift** | with **Alt** |
|---|---|---|---|
| CW / SSB / Data | 10 Hz | 100 Hz | 1 kHz |
| AM | 100 Hz | 1 kHz | 10 kHz |
| NFM | 1 kHz | 5 kHz | 25 kHz |
| WFM (broadcast) | 10 kHz | 100 kHz | 1 MHz |

(↑/↓ move the LO in larger, mode-appropriate steps.)

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

## Transmitting — SSB (voice)

1. Set the mode to **USB** or **LSB**.
2. In **Radio Control → Transmit**, choose your **Microphone** and set **Mic Gain**; watch the level
   meter and keep the **clip** indicator off on voice peaks.
3. **Hold the Space bar to transmit** (push-to-talk); release to receive.

Optional **TX processing** (under **Advanced**): a soft **limiter** (peak protection) and a **speech
compressor** (more average talk power). Leave the limiter on; add compression to taste.

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
- Operator settings are **locked while connected** to a server (to avoid surprise changes mid-session).

## Understanding the control sections

A consistent rule governs the collapsible sections:

- **Status** — read-only information (telemetry, meters).
- **Diagnostics** — system-testing tools that key/exercise the rig (Two-Tone, Spot/SWR, SWR Sweep,
  TX Test Tone). Not normal operation.
- **Advanced** — normal-operation controls you rarely change (TX limiter/compressor, WSJT-X setup).

Diagnostics and Advanced are hidden until you tick **"Show advanced & diagnostics controls."**

---

If something doesn't work as expected, see **[Troubleshooting](troubleshooting.md)**.
