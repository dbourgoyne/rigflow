use std::fs;
use std::path::PathBuf;

use rigflow_server::server::discovery::{debug_print_discovered_radios, discover_radios, DiscoveryConfig};

#[test]
fn discovers_wav_radios_and_fake_tone() {
    let temp = tempfile::tempdir().unwrap();
    let dir: PathBuf = temp.path().to_path_buf();

    fs::write(dir.join("sample_a.wav"), b"fake wav a").unwrap();
    fs::write(dir.join("sample_b.WAV"), b"fake wav b").unwrap();
    fs::write(dir.join("ignore.txt"), b"not a wav").unwrap();

    let config = DiscoveryConfig {
        wav_dir: Some(dir),
    };

    let radios = discover_radios(&config);
    debug_print_discovered_radios(&radios);

    let names: Vec<String> = radios.iter().map(|r| r.display_name.clone()).collect();

    assert!(names.iter().any(|n| n == "Fake Tone"));
    assert!(names.iter().any(|n| n.contains("sample_a.wav")));
    assert!(names.iter().any(|n| n.contains("sample_b.WAV")));

    let wav_count = radios
        .iter()
        .filter(|r| matches!(r.hardware_kind, rigflow_core::radio::HardwareKind::WavFile))
        .count();

    assert_eq!(wav_count, 2);
}
