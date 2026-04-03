use std::fs;
use std::path::{Path, PathBuf};

use rigflow_core::radio::{
    HardwareKind, RadioCapabilities, RadioDescriptor, RadioId,
};
use crate::server::config::{ServerConfig};

pub fn discover_radios(config: &ServerConfig) -> Vec<RadioDescriptor> {
    let mut radios = Vec::new();
    let path_option = Some(Path::new(&config.wav_dir));

    radios.extend(discover_rtl_radios());
    radios.extend(discover_wav_radios(path_option));
    radios.push(build_fake_tone_radio());

    radios
}

fn discover_rtl_radios() -> Vec<RadioDescriptor> {
    // TODO: adapt this to your actual RTL enumeration API.
    //
    // The goal for this baby step is:
    // - if one RTL-SDR is plugged in, return exactly one descriptor
    // - if none are plugged in, return none
    //
    // Replace the placeholder block below with your real enumeration.
    //
    // Example shape:
    // let devices = crate::hardware::rtl_sdr::list_devices().unwrap_or_default();
    // devices.into_iter().enumerate().map(|(idx, dev)| { ... }).collect()

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

fn try_discover_rtl_count() -> Result<usize, String> {
    // ----- REPLACE THIS WITH YOUR REAL RTL ENUMERATION -----
    //
    // If you already have something in:
    //   - src/hardware/rtl_sdr.rs
    //   - src/bin/rtlsdr_list.rs
    //
    // hook that in here.
    //
    // Temporary placeholder:
    Ok(1)
}

fn discover_wav_radios(wav_dir: Option<&Path>) -> Vec<RadioDescriptor> {
    let Some(dir) = wav_dir else {
        return Vec::new();
    };

    let mut radios = Vec::new();

    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            eprintln!("Failed to read wav dir '{}': {err}", dir.display());
            return radios;
        }
    };

    let mut wav_paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| is_wav_file(path))
        .collect();

    wav_paths.sort();

    for (idx, path) in wav_paths.into_iter().enumerate() {
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown.wav")
            .to_string();

        radios.push(RadioDescriptor {
            id: RadioId(format!("wav:{idx}")),
            display_name: format!("WAV {}", file_name),
            hardware_kind: HardwareKind::WavFile,
            index: idx as u32,
            serial: Some(path.display().to_string()),
            capabilities: default_radio_capabilities(),
        });
    }

    radios
}

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

fn is_wav_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|s| s.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("wav"))
            .unwrap_or(false)
}

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
