//! Keeps the audio backends' C libraries from writing diagnostics to stderr.
//!
//! cpal's ALSA backend probes every PCM hint with `snd_pcm_open` while enumerating
//! devices. On machines with alsa-plugins installed those hints also cover jack,
//! oss, dmix and dsnoop, and when such a probe fails alsa-lib and libjack write the
//! error straight to stderr, bypassing this process's logger. Run as a systemd
//! service, journald collects those lines: a single enumeration produced dozens of
//! them (measured).
//!
//! Both libraries expose replacement hooks: `snd_lib_error_set_handler` in alsa-lib
//! and `jack_set_error_function` / `jack_set_info_function` in libjack. They are
//! replaced with no-ops here. No diagnostic information is lost - the failure reason
//! still reaches cpal's return value (`AppError::Audio`) and the `cpal-output` log
//! target, which are this process's own logs; the C libraries' English stderr output
//! has no business being mixed into them.
//!
//! **Release builds only**: debug builds keep the raw C library output, because
//! those lines are often the most direct clue when investigating devices, formats or
//! plugins. To reproduce the quiet release behaviour in a debug build, drop the
//! `cfg!(debug_assertions)` check below.

#[cfg(target_os = "linux")]
mod linux {
    use std::ffi::{c_char, c_int, c_void, CString};
    use std::sync::Once;

    // Declared with raw pointer parameters only: the handler has to ignore every
    // argument, so its variadic signature is not reproduced here.
    extern "C" {
        fn snd_lib_error_set_handler(handler: *mut c_void) -> c_int;
        fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    const RTLD_NOW: c_int = 2;

    type JackMessageFunction = unsafe extern "C" fn(*const c_char);

    static ONCE: Once = Once::new();

    unsafe extern "C" fn ignore_alsa_error(
        _file: *const c_char,
        _line: c_int,
        _function: *const c_char,
        _err: c_int,
        _format: *const c_char,
    ) {
    }

    unsafe extern "C" fn ignore_jack_message(_message: *const c_char) {}

    pub fn silence_library_diagnostics() {
        if cfg!(debug_assertions) {
            return;
        }
        ONCE.call_once(|| unsafe {
            let handler = ignore_alsa_error
                as unsafe extern "C" fn(*const c_char, c_int, *const c_char, c_int, *const c_char);
            snd_lib_error_set_handler(handler as *mut c_void);
            silence_jack();
        });
    }

    /// libjack is only loaded once ALSA's jack plugin is probed. Grab the same
    /// in-process instance by soname first, then replace its error and info output
    /// (both jack2 and pipewire-jack export these symbols).
    unsafe fn silence_jack() {
        let soname = CString::new("libjack.so.0").expect("soname has no NUL");
        let handle = dlopen(soname.as_ptr(), RTLD_NOW);
        if handle.is_null() {
            return;
        }
        for symbol in [c"jack_set_error_function", c"jack_set_info_function"] {
            let address = dlsym(handle, symbol.as_ptr());
            if address.is_null() {
                continue;
            }
            let setter: unsafe extern "C" fn(JackMessageFunction) = std::mem::transmute(address);
            setter(ignore_jack_message);
        }
    }
}

/// Idempotent. Other platforms have no such libraries, so the empty implementation
/// keeps call sites platform-agnostic.
#[cfg(target_os = "linux")]
pub use linux::silence_library_diagnostics;

#[cfg(not(target_os = "linux"))]
pub fn silence_library_diagnostics() {}
