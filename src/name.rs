#[cfg(target_os = "linux")]
use std::ffi::CStr;
use std::ffi::CString;

use crate::NameError;

#[derive(Clone, Debug)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) struct Name(CString);

impl TryFrom<&str> for Name {
    type Error = NameError;

    fn try_from(name: &str) -> Result<Self, Self::Error> {
        Ok(Self(CString::new(name)?))
    }
}

impl Name {
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn as_str(&self) -> &str {
        self.0
            .to_str()
            .expect("Name is only constructed from &str, so it is valid UTF-8")
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn as_c_str(&self) -> &CStr {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_from_rejects_interior_nul() {
        assert!(matches!(
            Name::try_from("bad\0name"),
            Err(NameError::ContainsNul(_))
        ));
    }

    #[test]
    fn try_from_rejects_leading_and_trailing_nul() {
        assert!(Name::try_from("\0name").is_err());
        assert!(Name::try_from("name\0").is_err());
    }

    #[test]
    fn as_str_round_trips() {
        let name = Name::try_from("midi-io client").expect("valid name");
        assert_eq!(name.as_str(), "midi-io client");
    }

    #[test]
    fn as_str_round_trips_non_ascii_utf8() {
        let name = Name::try_from("sinté 🎹").expect("valid name");
        assert_eq!(name.as_str(), "sinté 🎹");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn as_c_str_is_nul_terminated_original() {
        let name = Name::try_from("alsa client").expect("valid name");
        assert_eq!(name.as_c_str(), c"alsa client");
    }
}
