use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::persistence::models::{DemodPreferenceSetFile, RadioSettingsFile};
use crate::sidetone::SidetoneShared;
use crate::ui::om_bands::LicenseClass;
use rigflow_core::dsp::modes::DeemphasisMode;
use rigflow_core::dsp::modes::{DemodMode, Sideband};
use rigflow_core::radio::RadioCapabilities;
use rigflow_core::radio::amplifier::AmplifierStatus;
use rigflow_core::radio::iq_recording::IqRecordingStatus;
use rigflow_core::radio::source_control::{SourceCapabilities, SourceControlState};
use rigflow_core::radio::source_status::SourceStatus;
use rigflow_core::radio::swr_sweep::{SwrSweepProgress, SwrSweepResult};
use rigflow_core::radio::tx_audio_diag::TxAudioDiag;
use rigflow_core::radio::tx_tune::TxTuneResult;

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

    /// Impulse noise blanker enabled (radio control).
    pub nb_enabled: bool,
    /// Noise-blanker level/sensitivity in [0.0, 1.0].
    pub nb_threshold: f32,
    /// Adaptive auto-notch enabled (nulls steady carriers) (radio control).
    pub notch_auto_enabled: bool,

    /// AGC (automatic gain control) — radio control.
    pub agc_enabled: bool,
    pub agc_strength: f32,

    /// S-meter (read-only status): uncalibrated relative dBm + S-units (0..=9).
    pub signal_dbm: f32,
    pub signal_s_units: i32,

    /// Receive-audio volume in percent (0–100).  Persisted per-operator.
    pub volume_percent: u8,

    /// Show the Advanced & Diagnostics controls (two-tone test, TX-audio
    /// diagnostics, limiter/compressor, digital interface).  Off by default for
    /// an uncluttered view; persisted per-operator.
    pub show_advanced: bool,

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

    /// Waterfall frame-rate debounce (continuous slider → server).
    pub waterfall_rate_debounce: DebounceState,

    /// Set on radio acquire so the panel pushes the restored waterfall rate to the
    /// server on connect (the server starts at its own default).
    pub pending_apply_waterfall_rate: bool,

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

    /// Waterfall frame rate in Hz sent to the server (0 = off). Persisted with the
    /// other waterfall display prefs; range 0–30 (matches the server clamp).
    pub waterfall_frame_rate_hz: f32,

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

    /// Attached amplifier status (Phase 1: HR50). `model: None` = no amplifier.
    pub amplifier_status: AmplifierStatus,

    /// Receive IQ recording status (Phase 1), from the server.
    pub iq_recording_status: IqRecordingStatus,

    /// Digital Audio Interface: whether the virtual audio endpoints were
    /// created/found at startup.  Informational only.
    /// `digital_rx_available` = `RigflowDigitalRX` source (apps record from);
    /// `digital_input_available` = `RigflowDigitalInput` sink (apps play TX to);
    /// `digital_output_available` = internal `RigflowDigitalOutput` sink.
    pub digital_output_available: bool,
    pub digital_rx_available: bool,
    pub digital_input_available: bool,

    /// Failure reason paired with each `digital_*_available` flag (set at startup
    /// when the endpoint could not be created — e.g. pactl missing / PipeWire
    /// down).  `None` when the endpoint is available or no reason was captured.
    pub digital_output_reason: Option<String>,
    pub digital_rx_reason: Option<String>,
    pub digital_input_reason: Option<String>,

    /// Hamlib NET rigctl (CAT) server status.  `Some(reason)` when the server
    /// could not bind its port (e.g. 4532 already in use); `None` when bound OK.
    pub rigctl_status: Option<String>,

    /// TCI server status.  `Some(reason)` when the server could not bind its port
    /// (e.g. 40001 already in use); `None` when bound OK.  Mirrors `rigctl_status`.
    pub tci_status: Option<String>,

    /// Digital Audio Interface (Phase 2): RX audio router to
    /// `RigflowDigitalOutput`.  Shared with the media thread; the UI toggles it.
    pub digital_rx: Arc<crate::digital_rx::DigitalRxOutput>,

    /// TCI server RX-audio tap.  Shared with the media thread (push) and the TCI
    /// server task (drain → WebSocket).  No-op until a TCI client streams.
    pub tci_rx_audio: Arc<crate::tci_server::TciRxAudio>,

    /// Live TX-audio diagnostics for SSB mic transmit (zero unless keyed).
    pub tx_audio_diag: TxAudioDiag,

    /// SSB two-tone test generator (diagnostic).  When enabled, the mic-TX
    /// path transmits `Tone A + Tone B` instead of mic audio (USB/LSB only).
    /// Not persisted — a transient calibration tool.
    pub two_tone_enabled: bool,
    pub two_tone_a_hz: f32,
    pub two_tone_b_hz: f32,
    pub two_tone_level_percent: u16,

    /// TX soft peak limiter (ALC Phase 1).  Enabled by default; threshold is a
    /// percent of full scale (50–99). Not persisted.
    pub tx_limiter_enabled: bool,
    pub tx_limiter_threshold_percent: u16,

    /// SSB speech compressor (before the limiter).  Disabled by default; level
    /// 0–10 (default 3). Not persisted.
    pub compressor_enabled: bool,
    pub compressor_level: u8,

    /// Persisted source-control settings keyed by radio ID string.
    /// Mirrors `OperatorSettingsFile::source_control_preferences`.
    pub source_control_preferences: HashMap<String, SourceControlState>,

    /// When `true`, `draw_source_control_panel` should re-send all source
    /// control values to the server (used after applying saved preferences
    /// on radio acquire).
    pub pending_apply_source_control: bool,

    /// Per-radio operating state (mode, filters, squelch/NR2/AGC, volume, CW
    /// sidetone/hang, waterfall), keyed by radio ID.  Mirrors
    /// `OperatorSettingsFile::radio_settings`.
    pub radio_settings: HashMap<String, RadioSettingsFile>,

    /// When `true`, the Radio Control replay block also re-sends the restored
    /// mode / sideband / squelch / NR2 / AGC to the server (set on acquire).
    pub pending_apply_radio_settings: bool,

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

    /// Whether the WSJT-X / FT8 setup helper window is open (transient).
    pub show_wsjtx_setup_window: bool,

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

    /// PTT commanded over CAT (rigctl `T 1`/`T 0`, e.g. WSJT-X).  Drives the
    /// status-bar TX indicator and the rigctl `t` readback.
    pub cat_ptt: bool,

    // ── Spectrum drag-pan momentum (client-local; never persisted) ───────
    /// Flick-momentum pan velocity in Hz/s.  0 = no momentum.  Seeded on a drag
    /// release with enough speed and decayed to 0 each frame by
    /// `advance_pan_momentum`.
    pub pan_velocity_hz_per_s: f32,
    /// Throttle timestamp for drag/momentum control-channel sends, so a 60 fps
    /// sweep doesn't flood the server with retunes.
    pub last_pan_send: Instant,
    /// Last LO (center) frequency actually sent during a drag/momentum pan, so a
    /// center change that lands in a throttled frame still reaches the server on
    /// the next allowed send.
    pub last_sent_center_hz: f32,

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
    /// Lock-free audio/latency metrics, published by the media runtime (RX jitter
    /// occupancy + network clock-offset / one-way latency) and read by the
    /// Latency panel.  Cloned (Arc) by the media runtime at startup.
    pub audio_metrics: Arc<crate::audio_metrics::AudioMetrics>,

    // ── Per-operator audio recording + voice keyer (client-only, on disk) ──
    /// RX-audio recorder sink slot, shared with the CPAL output callback; `Some`
    /// while recording.  Installed/cleared by the UI thread.
    pub rx_rec_slot: Arc<Mutex<Option<crate::audio_recorder::AudioRecorderSink>>>,
    /// Live RX-audio recording status (mirrors the owning recorder each frame).
    pub rx_audio_rec_status: crate::audio_recorder::AudioRecordingStatus,
    /// Live voice-keyer clip recording status (mirrors its recorder each frame).
    pub clip_rec_status: crate::audio_recorder::AudioRecordingStatus,
    /// True while a clip is being recorded from the mic (UI gate / indicator).
    pub clip_recording: bool,
    /// Name being entered for a new voice-keyer clip recording.
    pub clip_name_input: String,
    /// Voice-keyer clip filenames found in the operator's clips dir (runtime).
    pub voice_keyer_clips: Vec<String>,
    /// Selected voice-keyer clip filename ("" = none).  Persisted per operator.
    pub voice_keyer_clip: String,
    /// Local clip preview buffer, shared with the CPAL output callback.
    pub clip_preview: Arc<crate::audio_recorder::ClipPreview>,
    /// Lock-free voice-keyer playback state (playing/abort/progress), shared with
    /// the keyer playback thread.
    pub voice_keyer: Arc<crate::voice_keyer::KeyerShared>,
    /// Last voice-keyer / clip error to surface in the UI (runtime).
    pub voice_keyer_error: Option<String>,

    // UI → update() action requests (runtime; processed and cleared each frame),
    // so the `&self` UI panels don't need `&mut self` for client-local actions.
    /// RX audio recording: `Some(true)` = start, `Some(false)` = stop.
    pub rx_rec_request: Option<bool>,
    /// Voice-keyer clip recording: `Some(true)` = start, `Some(false)` = stop.
    pub clip_rec_request: Option<bool>,
    /// Clip preview: `Some(true)` = preview selected clip, `Some(false)` = stop.
    pub clip_preview_request: Option<bool>,
    /// Delete the selected voice-keyer clip.
    pub clip_delete_request: bool,
    /// Transmit the selected voice-keyer clip.
    pub voice_keyer_play_request: bool,
    /// Abort the voice keyer from the UI button.
    pub voice_keyer_abort_request: bool,
    /// The selected clip changed in the UI and should be persisted.
    pub voice_keyer_clip_dirty: bool,
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
            nb_enabled: false,
            nb_threshold: 0.5,
            notch_auto_enabled: false,
            agc_enabled: true,
            agc_strength: 0.5,
            signal_dbm: -140.0,
            signal_s_units: 0,
            volume_percent: 50,
            show_advanced: false,
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

            waterfall_rate_debounce: DebounceState::new(20.0),
            pending_apply_waterfall_rate: false,

            // =================================================================
            // CONNECTION / SERVER STATE
            // =================================================================
            rigflow_server_ip: "127.0.0.1".to_string(),
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
            waterfall_frame_rate_hz: 20.0,

            pan_velocity_hz_per_s: 0.0,
            last_pan_send: Instant::now(),
            last_sent_center_hz: 0.0,

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
            amplifier_status: AmplifierStatus::default(),
            iq_recording_status: IqRecordingStatus::default(),
            digital_output_available: false,
            digital_rx_available: false,
            digital_input_available: false,
            digital_output_reason: None,
            digital_rx_reason: None,
            digital_input_reason: None,
            rigctl_status: None,
            tci_status: None,
            digital_rx: crate::digital_rx::DigitalRxOutput::new(),
            tci_rx_audio: crate::tci_server::TciRxAudio::new(),
            tx_audio_diag: TxAudioDiag::default(),
            two_tone_enabled: false,
            two_tone_a_hz: 700.0,
            two_tone_b_hz: 1900.0,
            two_tone_level_percent: 50,
            tx_limiter_enabled: true,
            tx_limiter_threshold_percent: 90,
            compressor_enabled: false,
            compressor_level: 3,
            source_control_preferences: HashMap::new(),
            pending_apply_source_control: false,
            radio_settings: HashMap::new(),
            pending_apply_radio_settings: false,

            tx_tune_running: false,
            last_tx_tune_result: TxTuneResult::default(),
            swr_sweep_start_mhz: 14.000_000,
            swr_sweep_stop_mhz: 14.350_000,
            swr_sweep_error: None,
            swr_sweep_result: None,
            swr_sweep_progress: None,
            show_swr_sweep_window: false,
            swr_sweep_csv_status: None,
            show_wsjtx_setup_window: false,
            tx_tone_enabled: false,
            tx_tone_usb: true,
            tx_tone_freq_hz: 1000.0,
            tx_tone_running: false,
            cw_key_down: false,
            ssb_ptt_down: false,
            cat_ptt: false,
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
            audio_metrics: crate::audio_metrics::AudioMetrics::new(),
            rx_rec_slot: Arc::new(Mutex::new(None)),
            rx_audio_rec_status: crate::audio_recorder::AudioRecordingStatus::default(),
            clip_rec_status: crate::audio_recorder::AudioRecordingStatus::default(),
            clip_recording: false,
            clip_name_input: String::new(),
            voice_keyer_clips: Vec::new(),
            voice_keyer_clip: String::new(),
            clip_preview: crate::audio_recorder::ClipPreview::new(),
            voice_keyer: crate::voice_keyer::KeyerShared::new(),
            voice_keyer_error: None,
            rx_rec_request: None,
            clip_rec_request: None,
            clip_preview_request: None,
            clip_delete_request: false,
            voice_keyer_play_request: false,
            voice_keyer_abort_request: false,
            voice_keyer_clip_dirty: false,
        };

        let prefs = state.demod_preferences.get(state.demod_mode);

        state.filter_bandwidth_hz = prefs.filter_bandwidth_hz;
        state.pitch_hz = prefs.pitch_hz;

        state.filter_bw_debounce = DebounceState::new(state.filter_bandwidth_hz);
        state.pitch_debounce = DebounceState::new(state.pitch_hz);

        state
    }
}

/// Severity of a surfaced [`Problem`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProblemSeverity {
    /// A subsystem the operator is relying on is broken (rendered red).
    Error,
    /// A subsystem that may not be in use is degraded/unavailable (orange).
    Warning,
}

/// A single user-facing problem surfaced in the status-bar badge and the
/// "Status / Problems" panel.  Built at render time by [`collect_problems`].
#[derive(Debug, Clone)]
pub struct Problem {
    pub severity: ProblemSeverity,
    /// Short subsystem label, e.g. "Server", "Amplifier", "rigctl".
    pub source: &'static str,
    /// Human-readable reason, including the underlying error where available.
    pub detail: String,
}

/// Translate the current [`UiState`] snapshot into the list of active problems.
///
/// Pure (no I/O, no locking): subsystems write their raw status into `UiState`,
/// and this is the single place that decides what counts as a problem, so the
/// status-bar badge count and the panel list always agree.  Errors are ordered
/// before warnings.
pub fn collect_problems(s: &UiState) -> Vec<Problem> {
    let mut errors: Vec<Problem> = Vec::new();
    let mut warnings: Vec<Problem> = Vec::new();

    // --- Errors -------------------------------------------------------------
    // Server connect/acquire/connection failures.  The idle "no server" and the
    // "connected"/"radio acquired" states are not problems — only status text
    // that reports a failure counts, so this is silent until a real failure.
    let server = s.server_status.to_ascii_lowercase();
    if server.contains("fail")
        || server.contains("error")
        || server.contains("already has a client")
    {
        errors.push(Problem {
            severity: ProblemSeverity::Error,
            source: "Server",
            detail: s.server_status.clone(),
        });
    } else if server.contains("reconnecting") || server.contains("re-acquiring") {
        // Transient: the client lost the link and is recovering on its own.
        // Surface as a non-alarming Warning (escalates to Error if it gives up).
        warnings.push(Problem {
            severity: ProblemSeverity::Warning,
            source: "Server",
            detail: s.server_status.clone(),
        });
    }
    // Amplifier serial open failure, or a previously-detected amp that stopped
    // responding.  (Auto-detect finding no amp is log-only, never set here, so
    // stations without an HR50 see nothing.)
    if let Some(err) = &s.amplifier_status.last_error {
        errors.push(Problem {
            severity: ProblemSeverity::Error,
            source: "Amplifier",
            detail: err.clone(),
        });
    }
    // The SDR stopped sending IQ (HL2 link blip, RTL dongle pulled, device
    // powered off, …).  Only while a radio is held (so stale status after a
    // drop doesn't linger); cleared automatically when RX resumes.
    if s.radio_acquired && s.source_status.device_responding == Some(false) {
        errors.push(Problem {
            severity: ProblemSeverity::Error,
            source: "Radio",
            detail: "not responding (no data from device)".to_string(),
        });
    }

    // --- Warnings -----------------------------------------------------------
    // rigctl (CAT) server bind failure — affects WSJT-X/digital users only.
    if let Some(reason) = &s.rigctl_status {
        warnings.push(Problem {
            severity: ProblemSeverity::Warning,
            source: "rigctl",
            detail: reason.clone(),
        });
    }
    // Digital audio interface (PipeWire/pactl) — collapse the endpoints into one
    // line with the first captured reason (they usually share a root cause).
    // PipeWire/Pulse only exists on Linux; on other platforms these endpoints are
    // intentionally unavailable (digital uses the TCI server, not virtual audio),
    // so they are not a problem to report.
    #[cfg(target_os = "linux")]
    {
        let mut digital_down: Vec<&str> = Vec::new();
        let mut digital_reason: Option<&String> = None;
        for (available, reason, name) in [
            (
                s.digital_output_available,
                &s.digital_output_reason,
                "RigflowDigitalOutput",
            ),
            (
                s.digital_rx_available,
                &s.digital_rx_reason,
                "RigflowDigitalRX",
            ),
            (
                s.digital_input_available,
                &s.digital_input_reason,
                "RigflowDigitalInput",
            ),
        ] {
            if !available {
                digital_down.push(name);
                if digital_reason.is_none() {
                    digital_reason = reason.as_ref();
                }
            }
        }
        if !digital_down.is_empty() {
            let reason = digital_reason
                .map(String::as_str)
                .unwrap_or("unavailable (PipeWire/pactl not running?)");
            warnings.push(Problem {
                severity: ProblemSeverity::Warning,
                source: "Digital Audio",
                detail: format!("{} unavailable: {reason}", digital_down.join(", ")),
            });
        }
    }
    // Microphone fallback / unavailable (already shown in Radio Control; also
    // aggregated here).
    if !s.mic_status.is_empty() {
        warnings.push(Problem {
            severity: ProblemSeverity::Warning,
            source: "Microphone",
            detail: s.mic_status.clone(),
        });
    }
    // Operator/bookmark persistence save/load issues.
    if !s.persistence_status.is_empty() {
        warnings.push(Problem {
            severity: ProblemSeverity::Warning,
            source: "Persistence",
            detail: s.persistence_status.clone(),
        });
    }

    errors.append(&mut warnings);
    errors
}

#[cfg(test)]
mod problem_tests {
    use super::*;

    #[test]
    fn healthy_state_has_no_problems() {
        let mut s = UiState::default();
        // Default has all three digital endpoints unavailable; mark them up so a
        // freshly-connected, healthy station reports clean.
        s.digital_output_available = true;
        s.digital_rx_available = true;
        s.digital_input_available = true;
        s.server_status = "connected, 1 radios available".to_string();
        assert!(collect_problems(&s).is_empty());
    }

    #[test]
    fn failures_surface_with_errors_before_warnings() {
        let mut s = UiState::default();
        s.digital_output_available = true;
        s.digital_rx_available = true;
        s.digital_input_available = true;
        s.server_status = "connect failed: connection refused".to_string();
        s.rigctl_status = Some("rigctl: cannot bind 127.0.0.1:4532 — in use".to_string());
        s.amplifier_status.last_error = Some("HR50 serial open failed".to_string());

        let problems = collect_problems(&s);
        assert_eq!(problems.len(), 3);
        // Errors (Server, Amplifier) come before the rigctl warning.
        assert_eq!(problems[0].severity, ProblemSeverity::Error);
        assert_eq!(problems[1].severity, ProblemSeverity::Error);
        assert_eq!(problems.last().unwrap().severity, ProblemSeverity::Warning);
        assert_eq!(problems.last().unwrap().source, "rigctl");
    }

    #[test]
    fn idle_not_connected_is_not_a_problem() {
        let mut s = UiState::default();
        s.digital_output_available = true;
        s.digital_rx_available = true;
        s.digital_input_available = true;
        s.server_status = "no server".to_string();
        assert!(collect_problems(&s).is_empty());
    }

    fn healthy_digital(s: &mut UiState) {
        s.digital_output_available = true;
        s.digital_rx_available = true;
        s.digital_input_available = true;
    }

    #[test]
    fn reconnecting_is_a_warning_not_an_error() {
        let mut s = UiState::default();
        healthy_digital(&mut s);
        s.server_status = "reconnecting (attempt 3)…".to_string();
        let problems = collect_problems(&s);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].severity, ProblemSeverity::Warning);
        assert_eq!(problems[0].source, "Server");
    }

    #[test]
    fn re_acquiring_is_a_warning() {
        let mut s = UiState::default();
        healthy_digital(&mut s);
        s.server_status = "re-acquiring radio…".to_string();
        let problems = collect_problems(&s);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].severity, ProblemSeverity::Warning);
    }

    #[test]
    fn re_acquire_give_up_is_an_error() {
        let mut s = UiState::default();
        healthy_digital(&mut s);
        s.server_status = "re-acquire failed: radio still busy".to_string();
        let problems = collect_problems(&s);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].severity, ProblemSeverity::Error);
        assert_eq!(problems[0].source, "Server");
    }

    #[test]
    fn device_not_responding_is_a_radio_error() {
        let mut s = UiState::default();
        healthy_digital(&mut s);
        s.radio_acquired = true;

        s.source_status.device_responding = Some(false);
        let problems = collect_problems(&s);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].severity, ProblemSeverity::Error);
        assert_eq!(problems[0].source, "Radio");

        // Responding / unknown is not a problem.
        s.source_status.device_responding = Some(true);
        assert!(collect_problems(&s).is_empty());
        s.source_status.device_responding = None;
        assert!(collect_problems(&s).is_empty());

        // Not held → not surfaced even if the last status said not-responding.
        s.radio_acquired = false;
        s.source_status.device_responding = Some(false);
        assert!(collect_problems(&s).is_empty());
    }

    #[test]
    fn server_already_has_a_client_is_an_error() {
        let mut s = UiState::default();
        healthy_digital(&mut s);
        s.server_status = "server already has a client".to_string();
        let problems = collect_problems(&s);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].severity, ProblemSeverity::Error);
        assert_eq!(problems[0].source, "Server");
    }
}
