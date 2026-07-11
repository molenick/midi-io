use crate::ValueError;

#[repr(transparent)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DataByte(u8);

impl DataByte {
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for DataByte {
    type Error = ValueError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value > 0x7F {
            Err(ValueError::DataByte(value))
        } else {
            Ok(Self(value))
        }
    }
}

impl From<DataByte> for u8 {
    fn from(value: DataByte) -> Self {
        value.0
    }
}

impl std::fmt::Display for DataByte {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_byte_try_from_rejects_high_bit() {
        assert_eq!(DataByte::try_from(0).map(DataByte::get), Ok(0));
        assert_eq!(DataByte::try_from(127).map(DataByte::get), Ok(127));
        assert_eq!(DataByte::try_from(128), Err(ValueError::DataByte(128)));
        assert_eq!(DataByte::try_from(0xFF), Err(ValueError::DataByte(0xFF)));
    }

    #[test]
    fn data_byte_is_one_byte() {
        assert_eq!(std::mem::size_of::<DataByte>(), 1);
    }
}
