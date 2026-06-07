//! Silence libasound's noisy stderr diagnostics.
//!
//! On Linux, CPAL probes device configurations through ALSA, and the route
//! plugin prints harmless messages such as
//! `find_matching_chmap … Found no matching channel map` directly to stderr via
//! libasound's default error handler.  They don't affect functionality but are
//! noise.  We replace the default handler with a no-op.  libasound is already
//! linked (via CPAL/alsa-sys), so no extra dependency is needed.

/// Install a no-op libasound error handler (Linux only; no-op elsewhere).
/// Call once at startup, before any audio device is opened.
#[cfg(target_os = "linux")]
pub fn silence_alsa_errors() {
    use std::os::raw::{c_char, c_int};

    // `snd_lib_error_handler_t` is variadic in C; Rust can't define a variadic
    // fn on stable, so the handler declares only the fixed args and ignores the
    // C varargs — the standard, widely-used pattern for muting ALSA spam.
    type ErrHandler =
        Option<unsafe extern "C" fn(*const c_char, c_int, *const c_char, c_int, *const c_char)>;

    unsafe extern "C" {
        fn snd_lib_error_set_handler(handler: ErrHandler) -> c_int;
    }

    unsafe extern "C" fn silent(
        _file: *const c_char,
        _line: c_int,
        _function: *const c_char,
        _err: c_int,
        _fmt: *const c_char,
    ) {
    }

    // SAFETY: libasound is linked; installing an error handler is process-global
    // and safe to call once at startup.
    unsafe {
        snd_lib_error_set_handler(Some(silent));
    }
}

/// No-op on non-Linux targets (no ALSA).
#[cfg(not(target_os = "linux"))]
pub fn silence_alsa_errors() {}
