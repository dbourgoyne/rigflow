//! DX-cluster configuration (Phase 3): which node, login callsign, and the
//! client-side display filter. Operator-scoped, persisted as `cluster.json` in
//! the operator directory (mirrors `callbook.json`). Editing is locked while
//! connected to a rigflow server (`UiState.config_locked`), like other operator
//! settings — enforced by the UI (Phase 5).
//!
//! The filter is **client-side only** for v1: `current_band_only` + a mode
//! whitelist, both driven by data already on [`DxSpot`]. Spotter-continent
//! filtering and a configurable TTL are deferred (see the design doc §7/§12).

use rigflow_core::radio::ham_band::HamBand;

use super::DxSpot;

/// A built-in cluster node offered in the picker. All are free, callsign-only
/// login; they carry the same peered global feed, so the choice is about
/// reliability/filtering, not content.
pub struct ClusterNode {
    pub name: &'static str,
    pub host: &'static str,
    pub port: u16,
}

/// The shortlist. VE7CC is the default (cleanest `set/filter` command set).
pub const NODES: &[ClusterNode] = &[
    ClusterNode {
        name: "VE7CC",
        host: "ve7cc.net",
        port: 23,
    },
    ClusterNode {
        name: "NC7J",
        host: "dxc.nc7j.com",
        port: 23,
    },
    ClusterNode {
        name: "W3LPL",
        host: "w3lpl.net",
        port: 7373,
    },
    ClusterNode {
        name: "HRD",
        host: "hrd.wa9pie.net",
        port: 8000,
    },
];

/// Per-operator DX-cluster configuration.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DxClusterConfig {
    /// Master on/off. When off, the thread stays disconnected.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Login callsign; empty = use the operator's callsign.
    #[serde(default)]
    pub call: String,
    /// Show only spots on the currently tuned band (the default, most-relevant
    /// view). When false, spots from all bands show.
    #[serde(default = "default_true")]
    pub current_band_only: bool,
    /// Mode whitelist (uppercase, matched against `DxSpot.mode_hint`); empty =
    /// all modes.
    #[serde(default)]
    pub modes: Vec<String>,
}

fn default_host() -> String {
    NODES[0].host.to_string()
}
fn default_port() -> u16 {
    NODES[0].port
}
fn default_true() -> bool {
    true
}

impl Default for DxClusterConfig {
    fn default() -> Self {
        DxClusterConfig {
            enabled: false,
            host: default_host(),
            port: default_port(),
            call: String::new(),
            current_band_only: true,
            modes: Vec::new(),
        }
    }
}

impl DxClusterConfig {
    /// The callsign to log in with: the explicit `call` if set, else the
    /// operator's callsign. Uppercased/trimmed; empty if neither is available.
    pub fn login_call(&self, operator_call: &str) -> String {
        let c = if self.call.trim().is_empty() {
            operator_call
        } else {
            &self.call
        };
        c.trim().to_ascii_uppercase()
    }

    /// The built-in node name matching the current host/port, or `None` for a
    /// custom entry (for showing the picker's current selection).
    pub fn matching_node(&self) -> Option<&'static str> {
        NODES
            .iter()
            .find(|n| n.host == self.host && n.port == self.port)
            .map(|n| n.name)
    }

    /// Whether a spot passes the display filter given the currently tuned band.
    pub fn show(&self, spot: &DxSpot, current_band: Option<HamBand>) -> bool {
        if self.current_band_only && spot.band != current_band {
            return false;
        }
        if !self.modes.is_empty() {
            match &spot.mode_hint {
                Some(m) if self.modes.iter().any(|x| x.eq_ignore_ascii_case(m)) => {}
                _ => return false,
            }
        }
        true
    }
}

const CONFIG_FILE: &str = "cluster.json";

pub fn load_config(operator_dir: &std::path::Path) -> DxClusterConfig {
    std::fs::read_to_string(operator_dir.join(CONFIG_FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_config(operator_dir: &std::path::Path, cfg: &DxClusterConfig) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(cfg).unwrap_or_default();
    std::fs::write(operator_dir.join(CONFIG_FILE), json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn spot(band: Option<HamBand>, mode: Option<&str>) -> DxSpot {
        DxSpot {
            dx_call: "TEST".into(),
            freq_hz: 14_050_000,
            spotter: "X".into(),
            comment: String::new(),
            time_utc: String::new(),
            mode_hint: mode.map(|m| m.to_string()),
            band,
            received_at: Instant::now(),
        }
    }

    #[test]
    fn current_band_only_hides_other_bands() {
        let cfg = DxClusterConfig {
            current_band_only: true,
            ..Default::default()
        };
        assert!(cfg.show(&spot(Some(HamBand::B20), None), Some(HamBand::B20)));
        assert!(!cfg.show(&spot(Some(HamBand::B40), None), Some(HamBand::B20)));
        // No current band → nothing band-matches.
        assert!(!cfg.show(&spot(Some(HamBand::B20), None), None));
    }

    #[test]
    fn all_bands_when_toggle_off() {
        let cfg = DxClusterConfig {
            current_band_only: false,
            ..Default::default()
        };
        assert!(cfg.show(&spot(Some(HamBand::B40), None), Some(HamBand::B20)));
        assert!(cfg.show(&spot(None, None), None));
    }

    #[test]
    fn mode_whitelist_filters() {
        let cfg = DxClusterConfig {
            current_band_only: false,
            modes: vec!["FT8".into(), "CW".into()],
            ..Default::default()
        };
        assert!(cfg.show(&spot(None, Some("CW")), None));
        assert!(cfg.show(&spot(None, Some("ft8")), None)); // case-insensitive
        assert!(!cfg.show(&spot(None, Some("RTTY")), None));
        assert!(!cfg.show(&spot(None, None), None)); // unknown mode excluded when a whitelist is set
    }

    #[test]
    fn login_call_falls_back_to_operator() {
        let mut cfg = DxClusterConfig::default();
        assert_eq!(cfg.login_call("w1abc"), "W1ABC");
        cfg.call = "K9XYZ".into();
        assert_eq!(cfg.login_call("w1abc"), "K9XYZ");
    }

    #[test]
    fn matching_node_identifies_builtin() {
        let cfg = DxClusterConfig::default();
        assert_eq!(cfg.matching_node(), Some("VE7CC"));
        let custom = DxClusterConfig {
            host: "my.node".into(),
            ..Default::default()
        };
        assert_eq!(custom.matching_node(), None);
    }
}
