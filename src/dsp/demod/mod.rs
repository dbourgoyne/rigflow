pub mod fm;
pub mod ssb;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sideband {
    Usb,
    Lsb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemodMode {
    Usb,
    Lsb,
    Wfm,
}
