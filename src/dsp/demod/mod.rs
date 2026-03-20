pub mod am;
pub mod fm;
pub mod ssb;
pub mod cw;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Sideband {
    Usb,
    Lsb,
}