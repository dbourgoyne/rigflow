# Troubleshooting

Common problems and fixes, by symptom. For setup steps see the
**[Installation guide](installation.md)**; for operation see the
**[Operator guide](operator-guide.md)**.

---

## Connecting & finding radios

**The client won't connect to the server.**
- Confirm `rigflow-server` is actually running on the radio host.
- Check the **server IP** in the client (use `127.0.0.1` only if both run on one machine).
- The client reaches the server on **TCP 9000** and **UDP 9001** — make sure both are open and the
  two machines are on the same LAN. A restrictive firewall on either host is the usual culprit.

**Server exits with "another rigflow-server appears to be running … port already in use."**
- Only **one server per host** is allowed (it owns the radios). Stop the other instance first
  (`pkill rigflow-server`), then start yours.

**The Radios list is empty, or a radio is missing.**
- Radios are discovered when the **server** starts, and they must be plugged into the **server** host
  (not the client). Use the **Rescan** button in the client's Radios section after connecting hardware.

**The Hermes Lite 2 isn't discovered** (direct Pi↔HL2 link, both powered on together).
- The HL2 boots faster than the Pi and requests its IP before the Pi's DHCP server is up, so it
  strands on the wrong subnet. **Power-cycle the HL2 once the server host is fully booted, then
  Rescan** — it appears immediately. The permanent fix is to give the HL2 a **static IP** on the
  wired subnet. (See the Release Notes "Known Issues" for the full explanation.)

**The Hermes Lite 2 isn't discovered, but other software (SparkSDR, Quisk) finds it.**
- Confirm first that the server and the HL2 really are on the same subnet, and that you can
  `ping` the HL2 from the server host.
- If they are, probe the radio directly by address:
  ```bash
  rigflow-server --hl2-ip 192.168.1.50      # your HL2's IP
  ```
  Discovery still broadcasts as usual; `--hl2-ip` just adds a direct probe on top. Use it when
  broadcast can't reach the radio — a routed segment, a VLAN, or an access point that filters
  broadcast traffic. Several addresses can be given comma-separated.
- Give the HL2 a **static IP** (or a DHCP reservation) if you rely on `--hl2-ip`, so the address
  doesn't move.

**Seeing what discovery actually did.**
- The server takes its log level from the `RUST_LOG` environment variable — there is no CLI flag:
  ```bash
  RUST_LOG=debug rigflow-server
  ```
  Discovery logs each address it probes (`HL2 discovery: request sent on eth0 (… → …)`) and every
  device that answers, which tells you whether the request is leaving on the interface you expect.

**The RTL-SDR isn't found.**
- The Linux TV-tuner driver grabs it by default — blacklist it
  (`blacklist dvb_usb_rtl28xxu`) and reboot, and make sure your user can access the USB device
  (install the `rtl-sdr` package's udev rules). See the [Installation guide](installation.md).

**Connected and the controls work, but the spectrum is flat and there's no audio.**
- The control plane and the media plane are separate. Control (TCP 9000) is clearly working if band
  changes reach the radio — so what's missing is the media stream, which the server sends to the
  **client** on **UDP 50000**.
- Open inbound UDP 50000 on the **client** host. Opening the server's ports is not enough; this is
  the most commonly missed step when server and client are on different machines. See the
  [Installation guide](installation.md) §5 for the full both-directions port list.

**A radio releases or the client reconnects on its own.**
- Rigflow **auto-reconnects** and re-acquires after a network blip — brief drops recover themselves.
- A radio can be held by **one client at a time**. If someone else acquires it, or a lease expires
  while idle, you'll be released; just re-acquire.

---

## Audio

**No receive audio.**
- Confirm a radio is **acquired** and the **waterfall is moving** (if it's moving, data is flowing and
  the issue is local audio).
- Raise **Volume** (Radio Control → Audio), and check your computer's audio **output device** is the
  one you're listening on.

**Audio is choppy or drops out.**
- This is a network/jitter symptom. Put the client and server on the **same LAN**, and make sure the
  client can receive **inbound UDP on port 50000** from the server (the media stream) — a firewall on
  the client host can silently drop it.

**DATA mode is much quieter than every other mode, and WSJT-X shows a low input level.**
- Expected, and not a fault. DATA bypasses the receive AGC to keep the path flat and linear for the
  decoder; every other mode gets an AGC lift that DATA deliberately does not. See
  [Signal path & expected behavior](signal-path.md).
- **A low reading is not worth chasing.** Removing AGC scales the signal and the noise by the same
  amount, so the signal-to-noise ratio is unchanged — and FT8 decoding depends on SNR, not on
  absolute level. Raising the level cannot win you decodes, and driving it too high can lose them,
  because clipping destroys the relative signal levels the decoder works from.
- To **monitor by ear**, raise **Volume** (Radio Control → Audio). That affects the speaker only.
  The level sent to a digital application is fixed on purpose, so your monitoring level can never
  change what the decoder receives.

**CWU and CWL sound the same on receive.**
- Expected — Rigflow doesn't reject the opposite side of a CW signal yet, so the two modes differ only
  in transmit, not receive. Not a bug. See [Signal path & expected behavior](signal-path.md).

---

## Transmitting

**Nothing happens when I press Space / no RF.**
- The Space bar only keys when **no text field has focus** — click away from any text box first.
- You must be in a **transmit-capable mode** (USB/LSB/CW…) on a TX-capable radio (the HL2), with the
  radio acquired.
- If the server reports **"TX inhibited by hardware,"** the rig itself is blocking TX — check the
  radio's state and connections.

**It keys but there's little or no power.**
- Check **TX Drive** (Source Control) and, for SSB, **Mic Gain** and the level meter. Confirm you're
  transmitting into an antenna or dummy load.

**Always:** verify your signal and that you're loaded into an antenna/load before keying. See the
[Disclaimer](../DISCLAIMER.md).

---

## Digital (WSJT-X / FT8)

FT8 works two ways: **virtual audio** (Linux only — PipeWire/PulseAudio) and **TCI** (Linux *and*
macOS — and the **only** method on macOS; experimental). The fixes below are grouped by method.

### Virtual-audio method (Linux)

**WSJT-X has no audio / can't find the devices.**
- Make sure the mode is **DATA** in Rigflow (that's what routes the audio).
- The device names must match exactly: input **`RigflowDigitalRX`**, output **`RigflowDigitalInput`**.
- The virtual devices need **PipeWire** (or PulseAudio) running on the client desktop. If you don't
  have it, use the **TCI** method instead — it needs no virtual audio.

**WSJT-X won't key the radio.**
- In WSJT-X, CAT = **Hamlib NET rigctl**, host/port **`127.0.0.1:4532`**, and **PTT = CAT**. The
  client provides that rig-control endpoint while connected.

### TCI method (Linux & macOS)

**No audio over TCI.**
- In WSJT-X: **Rig = TCI**, **TCI Server = `127.0.0.1:40001`**, tick **Use TCI Audio**, and set
  **Audio → Input/Output** to the **TCI** device.
- Use a TCI-capable app (WSJT-X 2.7+, JTDX, MSHV). TCI support is experimental.

The in-app **WSJT-X / FT8 Setup** window (Radio Control → Advanced) shows the exact values for your
platform, with a live status for each piece.

---

## Amplifier (Hardrock-50)

**The HR50 isn't detected.**
- It connects by **USB serial to the server host**, and your user must be in the **`dialout`** group
  (log out and back in after adding it).
- The HR50's **own serial menu** must be enabled at a baud that matches Rigflow
  (`--hr50-serial auto` scans; or force `--hr50-serial /dev/ttyUSB0:19200`).
- **Using the amp's ACC port instead of USB?** Its baud is set by a *different* HR50 menu item than
  the USB port — set the ACC baud and make sure the FT-817-emulation option that disables ACC serial
  is **off**. (See the Hardrock-50 manual for the exact menu numbers.)

---

## Logging, callbook & DX cluster

**A contact has the wrong `MY_CNTY` / `MY_ITU_ZONE` / grid (or tqsl rejects it).**
- Your station details are **snapshotted into each contact when you log it**, so fixing the Station
  panel only affects *future* contacts. Correct an already-logged contact in the **Contacts view**
  (edit the fields), or bulk-fix in the database (`rigflow_log.db` — see
  [Where your data is stored](operator-guide.md#where-your-data-is-stored) for its per-platform path).
- **tqsl** wants US counties as `STATE,County` (e.g. `MD,Carroll`), not a bare county name — set it
  that way in the Station panel.

**Callbook isn't filling name / grid.**
- Open the **Callbook** window and confirm the provider is **enabled** and its credentials are
  entered. **QRZ** needs an **XML Logbook Data** subscription and uses your **qrz.com login** — not
  the QRZ Logbook API key used for QSO sync. **Callook** only knows **US** callsigns.
- With no provider (or offline), only the **offline prefix baseline** fills in — that's country +
  DXCC/zones, not name/city. The "via …" note tells you which source answered.

**Import made duplicates instead of marking contacts confirmed.**
- Use **Import** (plan → preview → commit); it matches on call/band/mode/date and applies
  confirmations rather than adding rows. A LoTW `lotwreport.adi` marks matching contacts confirmed.

**Export "Browse…" does nothing.**
- The file picker needs a desktop **file-chooser portal** (`xdg-desktop-portal` with a backend). If
  your session has none, type the output path into the field by hand instead.

**Service/callbook credentials aren't remembered.**
- On Linux they're stored in the desktop **secret service** (GNOME Keyring / KWallet). With none
  running, Rigflow falls back to an **encrypted file** — that's expected, not an error.

**DX cluster says "connected" but I see no spots.**
- Check the **"N received · M shown"** line. **0 received** on a busy band → wrong host/port, or your
  callsign was rejected at login; try another node. **Received but 0 shown** → the display filter is
  hiding them: turn off **current band only** or clear the **mode** filter in Configure.
- Markers only draw for spots **inside the visible span**; use the **spot list** to see the whole band
  and click a row to tune.

---

## Settings & display

**My settings won't change / fields are greyed out.**
- Operator/library settings are **locked while connected** to a server. Disconnect to edit them.

**My settings reset themselves once.**
- If a settings file is corrupted, Rigflow **quarantines it and starts fresh** rather than refusing to
  run. A one-time reset after a crash/bad write is this recovery working as intended.

**The S-meter / power / SWR readings look off.**
- They're **approximate by design** — good for relative judgements (comparing signals, spotting high
  SWR, peaking a tune), not lab-calibrated absolute values. Not a bug. See
  [Signal path & expected behavior](signal-path.md).

## Capturing diagnostics with rigflow-probe

For hard-to-pin-down problems (audio dropouts, waterfall stutter), a maintainer may ask you to
capture data with **`rigflow-probe`** — a small headless tool that connects to the server like the
client does, but with no GUI and no speakers. It records the received audio to a WAV and prints
transport statistics (packet loss, jitter-buffer events, server pacing), which helps separate a
server/DSP issue from a network one.

It isn't shipped as a prebuilt binary — build it from source. It needs the
[Rust toolchain](installation.md) but, unlike the client, **no audio libraries**:

```bash
cargo build --profile dist -p rigflow-probe
```

Then run it against your server (use `127.0.0.1` if the probe and server are on the same machine),
pointing at the radio and frequency of interest:

```bash
./target/dist/rigflow-probe --radio hl2 --target-hz 14074000 --mode usb \
  --server <server-ip> --duration 30 --wav capture.wav
```

It prints a summary on exit and writes `capture.wav`; `--help` lists all options. The server is
single-client, so disconnect the GUI client first. Share the printed summary (and the WAV, if
asked) on the issue.

---

Still stuck? Check the server's console log (it reports discovery, leases, and TX faults), and the
client's status console at the bottom of the left panel.

If that doesn't resolve it, please open a
[GitHub issue](https://github.com/dbourgoyne/rigflow/issues/new/choose) using the **Bug report**
template and include your version, platform, radio, and the relevant log lines (the form prompts
for each). Reports go through GitHub Issues — there's no support email.
