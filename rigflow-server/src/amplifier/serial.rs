//! Full-duplex serial transport for the amplifier (Phase 1).
//!
//! Opens a serial device (the Raspberry Pi USB link to the HR50) and configures
//! it raw 8N1 at the requested baud (HR50 default 9600).  Implemented directly
//! on `libc` termios to avoid the `serialport` crate's `libudev` dependency,
//! which doesn't build in all environments; the deployment target (the Pi) is
//! Linux, so `libc` is sufficient and portable.
//!
//! Assumes the HR50 does not echo commands (it is a CAT slave that replies in
//! upper case), so a response is the bytes up to the first `;`.

use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::io::RawFd;
use std::path::Path;
use std::time::{Duration, Instant};

use super::transport::AmplifierTransport;

/// USB identity + device path of an enumerated serial port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialPortInfo {
    /// Device node, e.g. `/dev/ttyUSB0`.
    pub path: String,
    /// USB vendor id of the owning device.
    pub vid: u16,
    /// USB product id of the owning device.
    pub pid: u16,
    /// USB product string, when the device exposes one.
    pub product: Option<String>,
}

/// Enumerate USB serial ports (`ttyUSB*` / `ttyACM*`) with the USB VID/PID of
/// the device that owns each one.
///
/// Reads sysfs directly (`/sys/class/tty/<name>/device` → walk up to the USB
/// node's `idVendor`/`idProduct`) so we keep the no-`libudev` policy the
/// hand-rolled termios transport exists to satisfy. This is identical on the
/// Raspberry Pi and x86_64 Linux. Ports with no USB parent (real UARTs) are
/// skipped; on non-Linux hosts `/sys/class/tty` is absent and the result is
/// empty.
pub fn enumerate_ports() -> Vec<SerialPortInfo> {
    let mut out = Vec::new();
    let Ok(dir) = fs::read_dir("/sys/class/tty") else {
        return out;
    };
    for entry in dir.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("ttyUSB") || name.starts_with("ttyACM")) {
            continue;
        }
        // `/sys/class/tty/<name>/device` symlinks into the USB device tree.
        let Ok(start) = fs::canonicalize(entry.path().join("device")) else {
            continue;
        };
        if let Some((vid, pid, product)) = read_usb_ids(&start) {
            out.push(SerialPortInfo {
                path: format!("/dev/{name}"),
                vid,
                pid,
                product,
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Walk up from a tty device node to the USB device that owns it and read its
/// `idVendor` / `idProduct` (and `product` string, when present). The interface
/// node nearest the tty doesn't carry the ids — they live a couple of levels up
/// on the USB device — so ascend until both files are found or we leave the USB
/// tree.
fn read_usb_ids(start: &Path) -> Option<(u16, u16, Option<String>)> {
    let mut cur = Some(start.to_path_buf());
    while let Some(dir) = cur {
        let vid_path = dir.join("idVendor");
        let pid_path = dir.join("idProduct");
        if vid_path.is_file() && pid_path.is_file() {
            let vid = read_hex_u16(&vid_path)?;
            let pid = read_hex_u16(&pid_path)?;
            let product = fs::read_to_string(dir.join("product"))
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            return Some((vid, pid, product));
        }
        // Stop at the sysfs root rather than scanning the whole filesystem.
        match dir.parent() {
            Some(p) if p != Path::new("/sys/devices") && p != Path::new("/") => {
                cur = Some(p.to_path_buf());
            }
            _ => return None,
        }
    }
    None
}

/// Parse a 4-hex-digit sysfs id file (e.g. `0403`) into a `u16`.
fn read_hex_u16(path: &Path) -> Option<u16> {
    let s = fs::read_to_string(path).ok()?;
    u16::from_str_radix(s.trim(), 16).ok()
}

/// A serial port opened read/write in non-blocking mode.
pub struct SerialTransport {
    fd: RawFd,
    /// Bytes received but not yet consumed as a complete (`;`-terminated) reply.
    buf: Vec<u8>,
}

impl SerialTransport {
    /// Open `path` (e.g. `/dev/ttyUSB0`) at `baud`, raw 8N1, no flow control.
    pub fn open(path: &str, baud: u32) -> io::Result<Self> {
        let speed = baud_to_speed(baud)?;
        let cpath = CString::new(path)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;

        let fd = unsafe {
            libc::open(
                cpath.as_ptr(),
                libc::O_RDWR | libc::O_NOCTTY | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        if let Err(e) = configure(fd, speed) {
            unsafe { libc::close(fd) };
            return Err(e);
        }

        Ok(Self {
            fd,
            buf: Vec::new(),
        })
    }
}

/// Apply raw 8N1 termios at `speed`.
fn configure(fd: RawFd, speed: libc::speed_t) -> io::Result<()> {
    unsafe {
        let mut tio: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut tio) != 0 {
            return Err(io::Error::last_os_error());
        }
        libc::cfmakeraw(&mut tio);
        tio.c_cflag |= libc::CLOCAL | libc::CREAD;
        tio.c_cflag &= !libc::CRTSCTS; // no hardware flow control
        tio.c_cflag &= !libc::PARENB; // no parity
        tio.c_cflag &= !libc::CSTOPB; // 1 stop bit
        tio.c_cflag &= !libc::CSIZE;
        tio.c_cflag |= libc::CS8; // 8 data bits
                                  // Non-blocking poll: return immediately with whatever is available.
        tio.c_cc[libc::VMIN] = 0;
        tio.c_cc[libc::VTIME] = 0;
        if libc::cfsetispeed(&mut tio, speed) != 0 || libc::cfsetospeed(&mut tio, speed) != 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::tcsetattr(fd, libc::TCSANOW, &tio) != 0 {
            return Err(io::Error::last_os_error());
        }
        libc::tcflush(fd, libc::TCIOFLUSH);
    }
    Ok(())
}

fn baud_to_speed(baud: u32) -> io::Result<libc::speed_t> {
    Ok(match baud {
        4800 => libc::B4800,
        9600 => libc::B9600,
        19200 => libc::B19200,
        38400 => libc::B38400,
        57600 => libc::B57600,
        115200 => libc::B115200,
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported baud {other}"),
            ));
        }
    })
}

impl AmplifierTransport for SerialTransport {
    fn write_cmd(&mut self, bytes: &[u8]) -> io::Result<()> {
        // Discard any stale/unsolicited input before sending so the upcoming
        // solicited reply is read cleanly — the HR50 emits unsolicited status on
        // band/PTT changes, and without this a single desync would persist and
        // time out every later poll (the reader could never re-sync).
        self.buf.clear();
        unsafe {
            libc::tcflush(self.fd, libc::TCIFLUSH);
        }
        let mut off = 0;
        while off < bytes.len() {
            let n = unsafe {
                libc::write(
                    self.fd,
                    bytes[off..].as_ptr() as *const libc::c_void,
                    bytes.len() - off,
                )
            };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::WouldBlock {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                }
                return Err(e);
            }
            off += n as usize;
        }
        Ok(())
    }

    fn read_response(&mut self, timeout: Duration) -> io::Result<Option<String>> {
        let deadline = Instant::now() + timeout;
        let mut tmp = [0u8; 256];
        loop {
            if let Some(pos) = self.buf.iter().position(|&b| b == b';') {
                let seg: Vec<u8> = self.buf.drain(..=pos).collect();
                return Ok(Some(String::from_utf8_lossy(&seg).into_owned()));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            let n =
                unsafe { libc::read(self.fd, tmp.as_mut_ptr() as *mut libc::c_void, tmp.len()) };
            if n > 0 {
                self.buf.extend_from_slice(&tmp[..n as usize]);
            } else if n == 0 {
                std::thread::sleep(Duration::from_millis(5));
            } else {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::WouldBlock {
                    std::thread::sleep(Duration::from_millis(5));
                } else {
                    return Err(e);
                }
            }
        }
    }
}

impl Drop for SerialTransport {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_hex_u16_parses_4_digit_ids() {
        let dir = std::env::temp_dir().join(format!("rigflow-hex-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("idVendor");
        fs::write(&f, "0403\n").unwrap();
        assert_eq!(read_hex_u16(&f), Some(0x0403));
        fs::write(&f, "04d8").unwrap();
        assert_eq!(read_hex_u16(&f), Some(0x04d8));
        fs::write(&f, "nothex").unwrap();
        assert_eq!(read_hex_u16(&f), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_usb_ids_walks_up_to_the_owning_device() {
        // Mimic sysfs: ids live a couple of levels above the tty interface node.
        let root = std::env::temp_dir().join(format!("rigflow-usb-{}", std::process::id()));
        let usb_dev = root.join("usb1/1-1");
        let interface = usb_dev.join("1-1:1.0/ttyUSB0");
        fs::create_dir_all(&interface).unwrap();
        fs::write(usb_dev.join("idVendor"), "0403\n").unwrap();
        fs::write(usb_dev.join("idProduct"), "6015\n").unwrap();
        fs::write(usb_dev.join("product"), "HR50 ACC cable\n").unwrap();

        let got = read_usb_ids(&interface);
        assert_eq!(
            got,
            Some((0x0403, 0x6015, Some("HR50 ACC cable".to_string())))
        );

        // A node with no USB ancestor yields nothing.
        let bare = root.join("bare/leaf");
        fs::create_dir_all(&bare).unwrap();
        assert_eq!(read_usb_ids(&bare), None);

        let _ = fs::remove_dir_all(&root);
    }
}
