use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::persistence::models::DemodPreferenceSetFile;
use crate::sidetone::SidetoneShared;
use crate::ui::om_bands::LicenseClass;
use rigflow_core::dsp::modes::DeemphasisMode;
use rigflow_core::dsp::modes::{DemodMode, Sideband};
use rigflow_core::radio::source_control::{SourceCapabilities, SourceControlState};
use rigflow_core::radio::source_status::SourceStatus;
use rigflow_core::radio::swr_sweep::{SwrSweepProgress, SwrSweepResult};
use rigflow_core::radio::tx_audio_diag::TxAudioDiag;
use rigflow_core::radio::tx_tune::TxTuneResult;
use rigflow_core::radio::RadioCapabilities;

/// A single CW memory macro: a short button label and the text to transmit.
#[derive(Debug, Clone, Default)]
pub struct CwMacro {
    pub label: String,
    pub text: String,
}

/// The 4 default CW macros, built from [`crate::cw_text::CW_MACRO_DEFAULTS`].
pub fn default_cw_macros() -> [CwMacro; 4] {
    crate::cw_text::CW_MACRO_DEFAULTS.map(|(label, text)| CwMacro {
        label: label.to_string(),
        text: text.to_string(),
    })
}

#[derive(Debug, Clone, Copy)]
pub struct DebounceState {
    pub last_sent_value: f32,
    pub last_send_time: Instant,
}
impl DebounceState {
    pub fn new(initial: f32) -> Self {
        Self {
            last_sent_value: initial,
            last_send_time: Instant::now(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct UiState {
    // =====================================================================
    // RADIO STATE (Operator-facing, synchronized with server)
    // =====================================================================
    /// Center frequency (LO), in Hz
    pub center_freq_hz: f32,

    /// Target tuned frequency, in Hz
    pub target_freq_hz: f32,

    /// Current demodulation mode
    pub demod_mode: DemodMode,

    /// Current sideband (SSB)
    pub sideband: Sideband,

    /// pitch (Hz)
    pub pitch_hz: f32,

    /// Audio filter bandwidth (Hz)
    pub filter_bandwidth_hz: f32,

    pub deemphasis_mode: DeemphasisMode,

    /// Receive squelch (radio control).  `squelch_open` is server-reported
    /// gate state (audio passing); the other two are operator controls.
    pub squelch_enabled: bool,
    pub squelch_threshold_db: f32,
    pub squelch_open: bool,

    /// NR2 spectral noise reduction enabled (radio control).
    pub nr2_enabled: bool,
    /// NR2 strength in [0.0, 1.0] (0 = none, 1 = max).
    pub nr2_strength: f32,

    /// AGC (automatic gain control) — radio control.
    pub agc_enabled: bool,
    pub agc_strength: f32,

    /// S-meter (read-only status): uncalibrated relative dBm + S-units (0..=9).
    pub signal_dbm: f32,
    pub signal_s_units: i32,

    /// Receive-audio volume in percent (0–100).  Persisted per-operator.
    pub volume_percent: u8,

    /// Input sample rate from SDR source (Hz)
    pub input_sample_rate_hz: f32,

    // =====================================================================
    // RADIO-DERIVED UI STATE
    // (depends on demod mode or radio state)
    // =====================================================================
    /// Tracks last demod mode for applying defaults (e.g. bandwidth)
    pub last_demod_mode_for_controls: Option<DemodMode>,

    /// One-shot flag: after radio acquire, reapply current mode controls
    pub pending_apply_mode_controls: bool,

    // =====================================================================
    // UI RUNTIME / HELPER STATE (non-persistent, non-radio)
    // =====================================================================
    /// Last filter bandwidth value sent to server (for debounce)
    pub filter_bw_debounce: DebounceState,

    /// Pitch debounce (shared across modes)
    pub pitch_debounce: DebounceState,

    // =====================================================================
    // CONNECTION / SERVER STATE
    // =====================================================================
    /// Server IP address entered by the user
    pub rigflow_server_ip: String,

    /// WebSocket port
    pub rigflow_server_ws_port: u16,

    /// UDP port (server)
    pub rigflow_server_udp_port: u16,

    /// Local UDP listen port
    pub udp_listen_port: u16,

    /// Whether connected to server
    pub server_connected: bool,

    /// Whether a radio is currently acquired
    pub radio_acquired: bool,

    /// Human-readable server status
    pub server_status: String,

    /// Available radios
    pub available_radios: Vec<rigflow_protocol::radio_control::RadioInfo>,

    /// Selected radio
    pub selected_radio_id: Option<String>,

    // =====================================================================
    // UI STATE (Rendering / Interaction)
    // =====================================================================
    pub runtime_error: String,

    pub selected_license: Option<LicenseClass>,

    pub spectrum_zoom_x: f32,

    // =====================================================================
    // WATERFALL / DISPLAY
    // =====================================================================
    pub adaptive_waterfall_normalization: bool,

    // persisted manual controls
    pub manual_waterfall_top_db: f32,
    pub manual_waterfall_range_db: f32,

    // runtime adaptive estimates, not persisted
    pub adaptive_top_db_estimate: f32,
    pub adaptive_floor_db_estimate: f32,
    pub adaptive_range_db_estimate: f32,

    pub display_zoom: f32,

    // =====================================================================
    // OPERATOR / PERSISTENCE (logical state, even if not yet persisted)
    // =====================================================================
    pub operator_id: String,
    pub known_operator_ids: Vec<String>,

    pub show_add_operator_dialog: bool,
    pub pending_operator_id: String,
    pub pending_operator_license: Option<LicenseClass>,

    pub show_delete_operator_dialog: bool,
    pub pending_delete_operator_id: Option<String>,

    pub persistence_status: String,

    // =====================================================================
    // PER-DEMOD OPERATOR PREFERENCES
    // =====================================================================
    pub demod_preferences: DemodPreferenceSetFile,

    // =====================================================================
    // BOOKMARKS
    // =====================================================================
    pub bookmarks: Vec<crate::persistence::BookmarkFile>,
    pub selected_bookmark_id: Option<String>,
    pub default_bookmark_id: Option<String>,
    pub auto_apply_default_bookmark_on_acquire: bool,

    pub show_add_bookmark_dialog: bool,
    pub pending_bookmark_name: String,
    pub pending_bookmark_notes: String,
    pub bookmark_status: String,
    pub pending_apply_default_bookmark: bool,

    // =====================================================================
    // SOURCE
    // =====================================================================
    pub source_control: SourceControlState,
    pub source_capabilities: SourceCapabilities,
    pub radio_capabilities: RadioCapabilities,
    /// Latest read-only telemetry from the active source.
    /// Empty (`SourceStatus::default()`) when the source does not report status.
    pub source_status: SourceStatus,

    /// Live TX-audio diagnostics for SSB mic transmit (zero unless keyed).
    pub tx_audio_diag: TxAudioDiag,

    /// Persisted source-control settings keyed by radio ID string.
    /// Mirrors `OperatorSettingsFile::source_control_preferences`.
    pub source_control_preferences: HashMap<String, SourceControlState>,

    /// When `true`, `draw_source_control_panel` should re-send all source
    /// control values to the server (used after applying saved preferences
    /// on radio acquire).
    pub pending_apply_source_control: bool,

    // =====================================================================
    // TX TUNE TEST (client-local; never persisted; never sent to server)
    // =====================================================================
    /// True while a TX tune test request has been sent and no result has
    /// arrived yet.  Used to disable the "Measure SWR" button and show a
    /// running indicator.  Never persisted.
    pub tx_tune_running: bool,

    /// Cached result from the most recent TX tune test measurement.
    /// `status = NotRun` until an actual tune test is executed.
    pub last_tx_tune_result: TxTuneResult,

    // ── SWR Sweep (HL2) ─────────────────────────────────────────────────
    /// Editable Start/Stop for the sweep, in MHz.
    pub swr_sweep_start_mhz: f64,
    pub swr_sweep_stop_mhz: f64,
    /// Client-side validation error to show under the Run button.
    pub swr_sweep_error: Option<String>,
    /// Latest sweep result and live progress (from the server).
    pub swr_sweep_result: Option<SwrSweepResult>,
    pub swr_sweep_progress: Option<SwrSweepProgress>,
    /// Whether the results popup is open, and the last CSV-export status line.
    pub show_swr_sweep_window: bool,
    pub swr_sweep_csv_status: Option<String>,

    // ── TX Test Tone (FDX Phase 2; client-local, not persisted) ─────────
    /// Master enable for the TX Test Tone section (shows the controls).
    pub tx_tone_enabled: bool,
    /// Sideband: `true` = USB (tone above carrier), `false` = LSB (below).
    pub tx_tone_usb: bool,
    /// Tone audio frequency in Hz.
    pub tx_tone_freq_hz: f32,
    /// True while a tone is transmitting (Start pressed, not yet stopped).
    pub tx_tone_running: bool,

    // ── CW keying (CW TX Phase 1; client-local, not persisted) ──────────
    /// Tracks whether the Space bar is currently keying CW, for edge detection
    /// (send StartCwKey on up→down, StopCwKey on down→up; no auto-repeat spam).
    pub cw_key_down: bool,

    /// Tracks whether the Space bar is currently keying SSB mic TX (USB/LSB),
    /// for edge detection (StartMicTx/StopMicTx).
    pub ssb_ptt_down: bool,

    // ── CW sidetone (client-local; never sent to server) ────────────────
    /// CW Sidetone Volume in percent (0–100), independent of RX Volume.
    pub cw_sidetone_volume: u8,

    /// CW semi-break-in hang time in ms (0–2000): how long PTT stays asserted
    /// after the last CW element before releasing.  Sent to the server.
    pub cw_hang_ms: u32,

    // ── Text-to-CW (client-side Morse; persisted per operator) ──────────
    /// The CW message text to send (also the "last used message").
    pub cw_message: String,
    /// Sending speed in words per minute (5–50).
    pub cw_speed_wpm: u32,
    /// CW memory macros (F1–F4): label + text, persisted per operator.
    pub cw_macros: [CwMacro; 4],

    /// Client-side CW decoder control + decoded-text output, shared lock-free
    /// with the media thread that runs the decoder.  Not persisted.
    pub cw_decode: Arc<crate::cw_decode::CwDecodeShared>,

    // ── Microphone capture (SSB Mic TX Phase 1; client-only, no RF) ─────
    /// Selected input device name ("" = system default).  Persisted.
    pub mic_device: String,
    /// Mic measurement gain in percent (0–200).  Persisted.
    pub mic_gain_percent: u16,
    /// Cached list of input device names for the dropdown (runtime only).
    pub mic_devices: Vec<String>,
    /// Status / fallback warning for the mic (runtime only).
    pub mic_status: String,
    /// UI-side decaying peak meter value (0.0–1.0+), updated each frame.
    pub mic_meter: f32,
    /// When set, the clip indicator stays lit until this instant (~500 ms hold).
    pub mic_clip_until: Option<Instant>,
    /// Lock-free mic level/clip/gain shared with the capture callback.
    pub mic_shared: Arc<crate::mic::MicShared>,
    /// Lock-free control state shared with the CPAL audio callback, which mixes
    /// the locally generated sidetone into the speaker output.  Cloned (Arc) by
    /// the media runtime at startup; written here from the Space-bar handler.
    pub sidetone: Arc<SidetoneShared>,
}

impl Default for UiState {
    fn default() -> Self {
        let mut state = Self {
            // =================================================================
            // RADIO STATE
            // =================================================================
            center_freq_hz: 0.0,
            target_freq_hz: 0.0,
            demod_mode: DemodMode::Wfm,
            sideband: Sideband::Lsb,

            demod_preferences: DemodPreferenceSetFile::default(),
            pitch_hz: 0.0,
            filter_bandwidth_hz: 3000.0,
            deemphasis_mode: DeemphasisMode::Off,
            squelch_enabled: false,
            squelch_threshold_db: -90.0,
            squelch_open: true,
            nr2_enabled: false,
            nr2_strength: 0.5,
            agc_enabled: true,
            agc_strength: 0.5,
            signal_dbm: -140.0,
            signal_s_units: 0,
            volume_percent: 50,
            input_sample_rate_hz: 0.0,

            // =================================================================
            // RADIO-DERIVED UI STATE
            // =================================================================
            last_demod_mode_for_controls: None,
            pending_apply_mode_controls: false,

            // =================================================================
            // UI RUNTIME / HELPER STATE
            // =================================================================
            filter_bw_debounce: DebounceState::new(0.0),

            pitch_debounce: DebounceState::new(0.0),

            // =================================================================
            // CONNECTION / SERVER STATE
            // =================================================================
            rigflow_server_ip: "192.168.0.225".to_string(),
            rigflow_server_ws_port: 9000,
            rigflow_server_udp_port: 9001,
            udp_listen_port: 0,

            server_connected: false,
            radio_acquired: false,

            server_status: "no server".to_string(),

            available_radios: Vec::new(),
            selected_radio_id: None,

            // =================================================================
            // UI STATE
            // =================================================================
            runtime_error: String::new(),
            selected_license: None,
            spectrum_zoom_x: 1.0,

            // =================================================================
            // WATERFALL / DISPLAY
            // =================================================================
            manual_waterfall_top_db: -35.0,
            manual_waterfall_range_db: 80.0,

            adaptive_waterfall_normalization: true,
            adaptive_top_db_estimate: -35.0,
            adaptive_floor_db_estimate: -140.0,
            adaptive_range_db_estimate: 100.0,

            display_zoom: 1.0,

            // =================================================================
            // OPERATOR / PERSISTENCE
            // =================================================================
            operator_id: String::new(),
            known_operator_ids: Vec::new(),

            show_add_operator_dialog: false,
            pending_operator_id: String::new(),
            pending_operator_license: None,

            show_delete_operator_dialog: false,
            pending_delete_operator_id: None,

            persistence_status: String::new(),

            // =================================================================
            // BOOKMARKS
            // =================================================================
            bookmarks: Vec::new(),
            selected_bookmark_id: None,
            default_bookmark_id: None,
            auto_apply_default_bookmark_on_acquire: false,

            show_add_bookmark_dialog: false,
            pending_bookmark_name: String::new(),
            pending_bookmark_notes: String::new(),
            bookmark_status: String::new(),
            pending_apply_default_bookmark: false,

            // =====================================================================
            // SOURCE
            // =====================================================================
            source_control: SourceControlState::default(),
            source_capabilities: SourceCapabilities::none(),
            radio_capabilities: RadioCapabilities::default(),
            source_status: SourceStatus::default(),
            tx_audio_diag: TxAudioDiag::default(),
            source_control_preferences: HashMap::new(),
            pending_apply_source_control: false,

            tx_tune_running: false,
            last_tx_tune_result: TxTuneResult::default(),
            swr_sweep_start_mhz: 14.000_000,
            swr_sweep_stop_mhz: 14.350_000,
            swr_sweep_error: None,
            swr_sweep_result: None,
            swr_sweep_progress: None,
            show_swr_sweep_window: false,
            swr_sweep_csv_status: None,
            tx_tone_enabled: false,
            tx_tone_usb: true,
            tx_tone_freq_hz: 1000.0,
            tx_tone_running: false,
            cw_key_down: false,
            ssb_ptt_down: false,
            cw_sidetone_volume: 25,
            cw_hang_ms: 300,
            cw_message: String::new(),
            cw_speed_wpm: 20,
            cw_macros: default_cw_macros(),
            cw_decode: Arc::new(crate::cw_decode::CwDecodeShared::default()),
            mic_device: String::new(),
            mic_gain_percent: 100,
            mic_devices: Vec::new(),
            mic_status: String::new(),
            mic_meter: 0.0,
            mic_clip_until: None,
            mic_shared: Arc::new(crate::mic::MicShared::default()),
            sidetone: Arc::new(SidetoneShared::default()),
        };

        let prefs = state.demod_preferences.get(state.demod_mode);

        state.filter_bandwidth_hz = prefs.filter_bandwidth_hz;
        state.pitch_hz = prefs.pitch_hz;

        state.filter_bw_debounce = DebounceState::new(state.filter_bandwidth_hz);
        state.pitch_debounce = DebounceState::new(state.pitch_hz);

        state
    }
}
