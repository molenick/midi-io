pub(crate) mod channel;
pub(crate) mod codec_error;
#[cfg(test)]
pub(crate) mod conformance;
pub(crate) mod data_byte;
pub(crate) mod decode;
pub(crate) mod message;
pub(crate) mod parse_error;
pub(crate) mod pitch_bend;
pub(crate) mod raw_message;
pub(crate) mod song_position;
#[cfg(any(
    all(
        feature = "io",
        any(
            target_os = "macos",
            target_os = "ios",
            target_os = "linux",
            target_arch = "wasm32"
        )
    ),
    test
))]
pub(crate) mod stream_parser;
pub(crate) mod sys_ex;
pub(crate) mod sys_ex_error;
pub(crate) mod value_error;
