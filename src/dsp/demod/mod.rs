pub mod am;
pub mod cw;
pub mod fm;
pub mod ssb;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Sideband {
    Usb,
    Lsb,
}
