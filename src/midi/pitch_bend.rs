use crate::midi::data_byte::DataByte;
use crate::ValueError;

pub(crate) const PITCH_BEND_CENTER: i16 = 8192;

#[repr(transparent)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct PitchBend(u16);

impl PitchBend {
    pub(crate) fn from_msb_lsb(msb: DataByte, lsb: DataByte) -> Self {
        Self(((msb.get() as u16) << 7) | lsb.get() as u16)
    }

    pub fn from_signed(value: i16) -> Result<Self, ValueError> {
        if !(-PITCH_BEND_CENTER..PITCH_BEND_CENTER).contains(&value) {
            Err(ValueError::PitchBend(value as i32))
        } else {
            Ok(Self((value as i32 + PITCH_BEND_CENTER as i32) as u16))
        }
    }

    pub const fn raw(self) -> u16 {
        self.0
    }

    pub const fn signed(self) -> i16 {
        self.0 as i16 - PITCH_BEND_CENTER
    }

    pub const fn lsb(self) -> u8 {
        (self.0 & 0x7F) as u8
    }

    pub const fn msb(self) -> u8 {
        ((self.0 >> 7) & 0x7F) as u8
    }
}

impl TryFrom<u16> for PitchBend {
    type Error = ValueError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if value > 0x3FFF {
            Err(ValueError::PitchBend(value as i32))
        } else {
            Ok(Self(value))
        }
    }
}

impl std::fmt::Display for PitchBend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.signed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitch_bend_center_is_zero_signed() {
        let center = PitchBend::from_signed(0).unwrap();
        assert_eq!(center.raw(), 8192);
        assert_eq!(center.signed(), 0);
        assert_eq!(center.lsb(), 0);
        assert_eq!(center.msb(), 64);
    }

    #[test]
    fn pitch_bend_signed_roundtrip() {
        for value in [-8192i16, -4096, -1, 0, 1, 4096, 8191] {
            assert_eq!(PitchBend::from_signed(value).unwrap().signed(), value);
        }
    }

    #[test]
    fn pitch_bend_from_signed_rejects_out_of_range() {
        assert_eq!(
            PitchBend::from_signed(-8192).map(PitchBend::signed),
            Ok(-8192)
        );
        assert_eq!(
            PitchBend::from_signed(8191).map(PitchBend::signed),
            Ok(8191)
        );
        assert_eq!(
            PitchBend::from_signed(8192),
            Err(ValueError::PitchBend(8192))
        );
        assert_eq!(
            PitchBend::from_signed(-8193),
            Err(ValueError::PitchBend(-8193))
        );
        assert_eq!(
            PitchBend::from_signed(i16::MAX),
            Err(ValueError::PitchBend(32767))
        );
        assert_eq!(
            PitchBend::from_signed(i16::MIN),
            Err(ValueError::PitchBend(-32768))
        );
    }

    #[test]
    fn pitch_bend_try_from_rejects_out_of_range() {
        assert_eq!(PitchBend::try_from(0).map(PitchBend::raw), Ok(0));
        assert_eq!(PitchBend::try_from(0x3FFF).map(PitchBend::raw), Ok(0x3FFF));
        assert_eq!(
            PitchBend::try_from(0x4000),
            Err(ValueError::PitchBend(0x4000))
        );
        assert_eq!(
            PitchBend::try_from(0xFFFF),
            Err(ValueError::PitchBend(0xFFFF))
        );
    }

    #[test]
    fn pitch_bend_is_two_bytes() {
        assert_eq!(std::mem::size_of::<PitchBend>(), 2);
    }
}
