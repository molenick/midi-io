use crate::SysExError;

/// Max Sysex size is limited to prevent denial-of-service through unlimited allocation.
pub(crate) const MAX_SYSEX_BYTES: usize = 1024 * 1024;

/// The amount of orphaned data bytes show as part of an error. Limited to prevent denial-of-service through unlimited allocation.
pub(crate) const ORPHAN_PREFIX_BYTES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SysEx(Vec<u8>);

impl TryFrom<&[u8]> for SysEx {
    type Error = SysExError;

    fn try_from(wire: &[u8]) -> Result<Self, Self::Error> {
        if wire.first() != Some(&0xF0) {
            return Err(SysExError::MissingStart);
        }
        if wire.len() < 2 || wire.last() != Some(&0xF7) {
            return Err(SysExError::Unterminated);
        }
        Self::new(&wire[1..wire.len() - 1])
    }
}

impl SysEx {
    pub fn new(body: &[u8]) -> Result<Self, SysExError> {
        if body.is_empty() {
            return Err(SysExError::EmptyBody);
        }
        if body.len() > MAX_SYSEX_BYTES {
            return Err(SysExError::TooLong {
                len: body.len(),
                max: MAX_SYSEX_BYTES,
            });
        }
        if let Some(index) = body.iter().position(|&b| b > 0x7F) {
            return Err(SysExError::HighBit {
                index,
                byte: body[index],
            });
        }
        Ok(Self(body.to_vec()))
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn to_wire_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.0.len() + 2);
        out.push(0xF0);
        out.extend_from_slice(&self.0);
        out.push(0xF7);
        out
    }
}

impl std::ops::Deref for SysEx {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<[u8]> for SysEx {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Display for SysEx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SysEx({} bytes)", self.0.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sysex_new_builds_from_unframed_body() {
        let sysex = SysEx::new(&[0x41, 0x10, 0x20]).unwrap();
        assert_eq!(sysex.bytes(), &[0x41, 0x10, 0x20]);
        assert_eq!(sysex.to_wire_bytes(), vec![0xF0, 0x41, 0x10, 0x20, 0xF7]);
    }

    #[test]
    fn sysex_new_rejects_empty_body() {
        assert_eq!(SysEx::new(&[]), Err(SysExError::EmptyBody));
    }

    #[test]
    fn sysex_new_rejects_high_bit_body() {
        assert_eq!(
            SysEx::new(&[0x41, 0x80, 0x10]),
            Err(SysExError::HighBit {
                index: 1,
                byte: 0x80
            })
        );
    }

    #[test]
    fn sysex_new_rejects_oversized_body() {
        let body = vec![0x01u8; MAX_SYSEX_BYTES + 1];
        assert_eq!(
            SysEx::new(&body),
            Err(SysExError::TooLong {
                len: MAX_SYSEX_BYTES + 1,
                max: MAX_SYSEX_BYTES
            })
        );
    }

    #[test]
    fn sysex_try_from_parses_wire_frame() {
        assert_eq!(
            SysEx::try_from([0xF0u8, 0x41, 0x10, 0xF7].as_slice())
                .unwrap()
                .bytes(),
            &[0x41, 0x10]
        );
    }

    #[test]
    fn sysex_try_from_requires_start_byte() {
        assert_eq!(
            SysEx::try_from([0x41u8, 0x10, 0xF7].as_slice()),
            Err(SysExError::MissingStart)
        );
    }

    #[test]
    fn sysex_try_from_requires_end_byte() {
        assert_eq!(
            SysEx::try_from([0xF0u8, 0x41, 0x10].as_slice()),
            Err(SysExError::Unterminated)
        );
    }

    #[test]
    fn sysex_try_from_requires_nonempty_body() {
        assert_eq!(
            SysEx::try_from([0xF0u8, 0xF7].as_slice()),
            Err(SysExError::EmptyBody)
        );
    }

    #[test]
    fn sysex_try_from_rejects_high_bit_body() {
        assert_eq!(
            SysEx::try_from([0xF0u8, 0x41, 0x80, 0xF7].as_slice()),
            Err(SysExError::HighBit {
                index: 1,
                byte: 0x80
            })
        );
    }

    #[test]
    fn sysex_try_from_rejects_oversized_body() {
        let mut wire = vec![0xF0u8];
        wire.extend(std::iter::repeat(0x01).take(MAX_SYSEX_BYTES + 1));
        wire.push(0xF7);
        assert_eq!(
            SysEx::try_from(wire.as_slice()),
            Err(SysExError::TooLong {
                len: MAX_SYSEX_BYTES + 1,
                max: MAX_SYSEX_BYTES
            })
        );
    }
}
