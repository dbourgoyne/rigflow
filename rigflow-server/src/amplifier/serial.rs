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
use std::io;
use std::os::unix::io::RawFd;
use std::time::{Duration, Instant};

use super::transport::AmplifierTransport;

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

    fn is_bidirectional(&self) -> bool {
        true
    }
}

impl Drop for SerialTransport {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}
