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

**The RTL-SDR isn't found.**
- The Linux TV-tuner driver grabs it by default — blacklist it
  (`blacklist dvb_usb_rtl28xxu`) and reboot, and make sure your user can access the USB device
  (install the `rtl-sdr` package's udev rules). See the [Installation guide](installation.md).

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
  client can receive **inbound UDP** from the server (the media stream) — a firewall on the client
  host can silently drop it.

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

---

Still stuck? Check the server's console log (it reports discovery, leases, and TX faults), and the
client's status console at the bottom of the left panel.
