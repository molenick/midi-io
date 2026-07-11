use crate::midi::data_byte::DataByte;
use crate::ValueError;

#[repr(transparent)]
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct SongPosition(u16);

impl SongPosition {
    pub(crate) fn from_msb_lsb(msb: DataByte, lsb: DataByte) -> Self {
        Self(((msb.get() as u16) << 7) | lsb.get() as u16)
    }

    pub const fn get(self) -> u16 {
        self.0
    }

    pub const fn lsb(self) -> u8 {
        (self.0 & 0x7F) as u8
    }

    pub const fn msb(self) -> u8 {
        ((self.0 >> 7) & 0x7F) as u8
    }
}

impl TryFrom<u16> for SongPosition {
    type Error = ValueError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if value > 0x3FFF {
            Err(ValueError::SongPosition(value))
        } else {
            Ok(Self(value))
        }
    }
}

impl std::fmt::Debug for SongPosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Display for SongPosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn song_position_splits_into_bytes() {
        let pos = SongPosition::try_from(256).unwrap();
        assert_eq!(pos.get(), 256);
        assert_eq!(pos.lsb(), 0);
        assert_eq!(pos.msb(), 2);
    }

    #[test]
    fn song_position_try_from_rejects_out_of_range() {
        assert_eq!(SongPosition::try_from(0).map(SongPosition::get), Ok(0));
        assert_eq!(
            SongPosition::try_from(0x3FFF).map(SongPosition::get),
            Ok(0x3FFF)
        );
        assert_eq!(
            SongPosition::try_from(0x4000),
            Err(ValueError::SongPosition(0x4000))
        );
        assert_eq!(
            SongPosition::try_from(0xFFFF),
            Err(ValueError::SongPosition(0xFFFF))
        );
    }
}
