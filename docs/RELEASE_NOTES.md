# Rigflow Release Notes

> Working document. Tracks user-facing changes, known issues, and workarounds as
> the project moves toward its first public release. Newest entries on top.

---

## Unreleased

### Changes

- **Subsystem failures are now shown on screen, not just in the log.** A new
  always-open **Status / Problems** section at the top of the left panel — plus a
  `⚠ N` badge in the top status bar (green "● OK" when clear) — lists active
  failures with their specific reason instead of failing silently. Covered:
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
