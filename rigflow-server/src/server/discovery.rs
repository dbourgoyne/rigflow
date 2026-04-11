use std::fs;
use std::path::{Path, PathBuf};

use rigflow_core::radio::{
    HardwareKind, RadioCapabilities, RadioDescriptor, RadioId,
};

use crate::server::config::ServerConfig;

/// Discover all radios available to the server.
///
/// Includes:
/// - RTL-SDR devices
/// - WAV file sources (from configured directory)
/// - Fake tone generator
pub fn discover_radios(config: &ServerConfig) -> Vec<RadioDescriptor> {
    let mut radios = Vec::new();

    radios.extend(discover_rtl_radios());
    radios.extend(discover_wav_radios(Path::new(&config.wav_dir)));
    radios.push(build_fake_tone_radio());

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
                    capabilities: default_radio_capabilities(),
                });
            }
        }
        Err(err) => {
            eprintln!("RTL discovery failed: {err}");
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
            eprintln!("Failed to read wav dir '{}': {err}", dir.display());
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
                capabilities: default_radio_capabilities(),
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
        capabilities: default_radio_capabilities(),
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
        supports_usb: true,
        supports_lsb: true,
    }
}

//
// ============================
// Debug
// ============================
//

pub fn debug_print_discovered_radios(radios: &[RadioDescriptor]) {
    println!("Discovered {} radios:", radios.len());

    for radio in radios {
        println!(
            "  id={} kind={:?} name='{}' serial={:?}",
            radio.id.0,
            radio.hardware_kind,
            radio.display_name,
            radio.serial
        );
    }
}
