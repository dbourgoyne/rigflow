use serde::{Deserialize, Serialize};

/// Which VFO a control or transmit refers to, for dual-VFO / split operation.
///
/// `A` is the primary VFO (the single-VFO mirror that all pre-dual-watch code
/// uses); `B` is the secondary VFO (independent frequency + mode, fed by the
/// source's second hardware receiver when dual-watch is active).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VfoSelect {
    #[default]
    A,
    B,
}

impl VfoSelect {
    /// The other VFO (for A↔B swap / "the receiving VFO is the non-TX one").
    pub fn other(self) -> Self {
        match self {
            VfoSelect::A => VfoSelect::B,
            VfoSelect::B => VfoSelect::A,
        }
    }
}
