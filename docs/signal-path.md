# Signal path & expected behavior

This page explains, in plain language, **what Rigflow does to your audio** on receive
and transmit — and which behaviors are **intentional**. If something seems surprising,
check here before assuming it's a bug: several of the most-reported "problems" are
working as designed.

For *how to operate*, see the **[Operator guide](operator-guide.md)**. For
*symptom → fix*, see **[Troubleshooting](troubleshooting.md)**.

All signal processing runs on the **server** (the machine your radio is plugged into).
The client just plays the audio, draws the waterfall, and sends your controls. Signal
levels — including the **S-meter and dBm/SWR readings — are relative, not calibrated**
(good for comparing signals and peaking a tune, not for absolute measurements).

---

## Receiving

Pick a **Demod** mode in **Radio Control → Receive** (WFM · NFM · AM · LSB · USB ·
**DATA** · CWU · CWL). The receive path is roughly:

**tune → filter the channel → demodulate → remove DC → AGC → audio filter →
(optional noise reduction) → (optional squelch) → speaker.**

The controls you steer it with:

- **Filter bandwidth** — how wide a slice of audio passes. Narrow it to cut adjacent
  signals, widen it for fidelity. Each mode remembers its own setting.
- **AGC** (automatic gain control) — evens out loud and weak signals. Strength **0 is
  effectively off**; higher rides the level harder.
- **NR2** (noise reduction) — reduces broadband hiss. Optional; off = untouched audio.
- **Squelch** — mutes the speaker until a signal is strong enough. Off by default.

---

## Transmitting — SSB (voice)

The microphone path is:

**mic → remove DC → (optional compressor) → limiter → SSB modulator → radio.**

- The **compressor** (optional, off by default) raises your *average* talk power.
- The **limiter** (on by default) is a safety catch that trims peaks just below
  clipping to keep your signal clean.
- **TX drive** (Source Control) sets the actual RF power.

Both the compressor and limiter live under **Radio Control → Audio → Advanced**, with
gauges showing how much they're working.

---

## CW

CW receive and transmit are **not** the same chain:

- **Receive** turns the Morse signal into an audible tone (a beat note at your chosen
  **pitch**). **CWU and CWL sound identical** — this is a **known limitation**, not a
  bug: Rigflow doesn't yet reject the opposite side of the CW signal, so the two modes
  differ only in where the transmit tone is placed, not in how receive sounds.
- **Transmit** generates the keyed carrier directly, with smooth rise/fall shaping to
  keep the keying clean (no clicks). The SSB compressor and limiter **do not apply to
  CW** — there's nothing to disable.

CW has its own extras: an on-air **CW decode** to text, a local **sidetone** for
monitoring, and keyboard/macro sending.

---

## Digital (FT8 / DATA)

In **DATA** mode the digital app (WSJT-X / JTDX / MSHV) owns the modem; Rigflow just
provides a clean, linear path and the audio transport. So in DATA mode Rigflow
**automatically**:

- **bypasses the TX compressor and limiter**, and
- **disables receive AGC**,

regardless of your SSB-voice settings — speech processing and a pumping AGC would only
hurt digital decoding. **Set your transmit level with TX drive** so the tone stays in
the clean region. You don't need to turn anything off by hand.

For setup (device names, CAT, TCI, Split), see **Digital modes** in the
**[Operator guide](operator-guide.md)**.

---

## Latency panel

The **Latency / Audio** panel measures Rigflow's own server↔client audio transport:
on **receive**, the network one-way delay (server → client) plus the client **jitter
buffer**, which is the *speaker* playout buffer; on **transmit**, the client mic ring
plus the server's mic queue.

What this means by mode:

- **SSB and CW** are fully represented — the jitter buffer is your actual receive
  (speaker) audio, and the mic ring + server queue are your actual transmit audio.
- **FT8 / digital** behaves the same whether you use the Linux PipeWire path **or**
  TCI (they reuse the same internal seams). The **network one-way** and the
  **transmit** figures (mic ring + server queue) apply. But FT8 receive audio is tapped
  *before* the jitter buffer on its way to WSJT-X, so the **jitter-buffer number
  reflects speaker monitoring, not the FT8 decode path**, and the panel does **not**
  include WSJT-X's own audio buffering or the final local hop to it (the PipeWire
  virtual device, or the TCI localhost socket).

The displayed values are smoothed and the CPAL sound-device buffers are not included,
so treat the totals as a close estimate rather than an exact figure.

---

## Known behaviors & limitations

These are **intentional** — please don't file them as bugs:

- **S-meter, dBm, and SWR are relative, not calibrated.** Use them for comparison and
  peaking, not absolute values.
- **CW receive sounds the same for CWU and CWL** (no opposite-side rejection yet).
- **DATA mode forces a clean path** — compressor, limiter, and receive AGC are off in
  DATA no matter what your voice settings say.
- **AGC is a single, basic algorithm** — one behavior, no fast/slow presets.
- **Receive/demod preferences and waterfall settings are saved per operator**, but
  **radio source controls (gain, PPM, direct sampling) are not persisted** — set them
  per session.
- **The two-tone test** (Transmit test aids) is a deliberate tool for checking
  transmit linearity, not a stuck tone.
