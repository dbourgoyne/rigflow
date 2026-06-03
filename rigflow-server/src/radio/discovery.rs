use log::{error, info};
use std::fs;
use std::path::{Path, PathBuf};

use rigflow_core::radio::source_control::{DirectSamplingMode, SourceCapabilities};
use rigflow_core::radio::{HardwareKind, RadioCapabilities, RadioDescriptor, RadioId};

use crate::config::ServerConfig;
use crate::radio::hl2_discovery;
use crate::source::hermeslite2::hl2_source_capabilities;

/// Discover all radios available to the server.
///
/// Includes:
/// - RTL-SDR devices
/// - WAV file sources (from configured directory)
/// - Fake tone generator
/// - Hermes Lite 2 devices (Protocol 1 UDP broadcast)
pub fn discover_radios(config: &ServerConfig) -> Vec<RadioDescriptor> {
    let mut radios = Vec::new();

    radios.extend(discover_rtl_radios());
    radios.extend(discover_wav_radios(Path::new(&config.wav_dir)));
    radios.push(build_fake_tone_radio());
    radios.extend(discover_hl2_radios());

    radios
}

//
// ============================
// RTL Discovery
// ============================
//

fn discover_rtl_radios() -> Vec<RadioDescriptor> {
    let mut radios = Vec::new();

    match try_discover_rtl_count() {
        Ok(count) => {
            for idx in 0..count {
                radios.push(RadioDescriptor {
                    id: RadioId(format!("rtl:{idx}")),
                    display_name: format!("RTL-SDR #{idx}"),
                    hardware_kind: HardwareKind::RtlSdr,
                    index: idx as u32,
                    serial: None,
                    radio_capabilities: default_radio_capabilities(),
                    source_capabilities: rtl_source_capabilities(),
                });
            }
        }
        Err(err) => {
            error!("RTL discovery failed: {err}");
        }
    }

    radios
}

/// Placeholder RTL enumeration.
///
/// Replace this with actual hardware enumeration logic.
fn try_discover_rtl_count() -> Result<usize, String> {
    // TODO: hook into real RTL-SDR enumeration
    Ok(1)
}

//
// ============================
// WAV Discovery
// ============================
//

fn discover_wav_radios(dir: &Path) -> Vec<RadioDescriptor> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            error!("Failed to read wav dir '{}': {err}", dir.display());
            return Vec::new();
        }
    };

    let mut wav_paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| is_wav_file(path))
        .collect();

    // Stable ordering ensures deterministic radio IDs (wav:0, wav:1, ...)
    wav_paths.sort();

    wav_paths
        .into_iter()
        .enumerate()
        .map(|(idx, path)| {
            let file_name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown.wav");

            RadioDescriptor {
                id: RadioId(format!("wav:{idx}")),
                display_name: format!("WAV {}", file_name),
                hardware_kind: HardwareKind::WavFile,
                index: idx as u32,
                serial: Some(path.display().to_string()),
                radio_capabilities: default_radio_capabilities(),
                source_capabilities: SourceCapabilities::none(),
            }
        })
        .collect()
}

fn is_wav_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|s| s.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("wav"))
            .unwrap_or(false)
}

//
// ============================
// Fake Radio
// ============================
//

fn build_fake_tone_radio() -> RadioDescriptor {
    RadioDescriptor {
        id: RadioId("fake:tone".to_string()),
        display_name: "Fake Tone".to_string(),
        hardware_kind: HardwareKind::FakeTone,
        index: 0,
        serial: None,
        radio_capabilities: default_radio_capabilities(),
        source_capabilities: SourceCapabilities::none(),
    }
}

//
// ============================
// Hermes Lite 2 Discovery
// ============================
//

fn discover_hl2_radios() -> Vec<RadioDescriptor> {
    hl2_discovery::discover_hl2_devices()
        .into_iter()
        .enumerate()
        .map(|(idx, dev)| RadioDescriptor {
            id: RadioId(format!("hl2:{}", dev.mac_hex())),
            display_name: format!("Hermes Lite 2 ({})", dev.addr.ip()),
            hardware_kind: HardwareKind::HermesLite2,
            index: idx as u32,
            // serial carries the IP:port so the worker can connect in step 4.
            serial: Some(dev.addr.to_string()),
            radio_capabilities: hl2_radio_capabilities(),
            source_capabilities: hl2_source_capabilities(),
        })
        .collect()
}

fn hl2_radio_capabilities() -> RadioCapabilities {
    RadioCapabilities {
        min_freq_hz: 10_000,
        max_freq_hz: 30_000_000,
        max_sample_rate_hz: 384_000,
        supports_wfm: false,
        supports_nfm: true,
        supports_am: true,
        supports_cw: true,
        supports_usb: true,
        supports_lsb: true,
    }
}

//
// ============================
// Capabilities
// ============================
//

fn default_radio_capabilities() -> RadioCapabilities {
    RadioCapabilities {
        min_freq_hz: 500_000,
        max_freq_hz: 1_700_000_000,
        max_sample_rate_hz: 2_400_000,
        supports_wfm: true,
        supports_nfm: true,
        supports_am: true,
        supports_cw: true,
        supports_usb: true,
        supports_lsb: true,
    }
}

fn rtl_source_capabilities() -> SourceCapabilities {
    SourceCapabilities {
        supports_sample_rate: true,
        sample_rates_hz: vec![1_024_000, 1_536_000, 2_048_000, 2_400_000],

        supports_gain_mode: true,
        supports_gain: true,
        gain_values_db: vec![
            0.0, 0.9, 1.4, 2.7, 3.7, 7.7, 8.7, 12.5, 14.4, 15.7, 16.6, 19.7, 20.7, 22.9, 25.4,
            28.0, 29.7, 32.8, 33.8, 36.4, 37.2, 38.6, 40.2, 42.1, 43.4, 43.9, 44.5, 48.0, 49.6,
        ],

        supports_ppm_correction: true,
        ppm_min: -100,
        ppm_max: 100,

        supports_direct_sampling: true,
        direct_sampling_modes: vec![
            DirectSamplingMode::Off,
            DirectSamplingMode::I,
            DirectSamplingMode::Q,
        ],
        direct_sampling_freq_hz_max: 30_000_000,

        tuner_freq_hz_min: 24_000_000,
        tuner_freq_hz_max: 1_766_000_000,

        supports_tx_tune_test: false,
            supports_band_control: false,
    }
}

//
// ============================
// Debug
// ============================
//

pub fn debug_print_discovered_radios(radios: &[RadioDescriptor]) {
    info!("Discovered {} radios:", radios.len());

    for radio in radios {
        info!(
            "  id={} kind={:?} name='{}' serial={:?}",
            radio.id.0, radio.hardware_kind, radio.display_name, radio.serial
        );
    }
}
