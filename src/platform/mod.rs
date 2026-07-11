mod common;
pub(crate) use common::PlatformClient;

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[path = "coremidi.rs"]
mod coremidi_backend;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use coremidi_backend::Backend;

#[cfg(target_os = "linux")]
#[path = "alsa.rs"]
mod alsa_backend;
#[cfg(target_os = "linux")]
use alsa_backend::Backend;

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "linux")))]
compile_error!("midi-io only supports macOS, iOS, and Linux");

macro_rules! log_error {
    ($($arg:tt)*) => {{
        #[cfg(feature = "tracing")]
        tracing::error!($($arg)*);
        #[cfg(not(feature = "tracing"))]
        let _ = format_args!($($arg)*);
    }};
}
pub(crate) use log_error;

macro_rules! log_warn {
    ($($arg:tt)*) => {{
        #[cfg(feature = "tracing")]
        tracing::warn!($($arg)*);
        #[cfg(not(feature = "tracing"))]
        let _ = format_args!($($arg)*);
    }};
}
pub(crate) use log_warn;

trait MutexExt<T> {
    fn lock_unpoisoned(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> MutexExt<T> for std::sync::Mutex<T> {
    fn lock_unpoisoned(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn map_send_err<T>(e: std::sync::mpsc::TrySendError<T>) -> crate::Error {
    match e {
        std::sync::mpsc::TrySendError::Full(_) => crate::IoError::BackendCommandChannelFull.into(),
        std::sync::mpsc::TrySendError::Disconnected(_) => crate::IoError::BackendThreadDied.into(),
    }
}
