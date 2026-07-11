use crate::ValueError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Channel {
    Ch1 = 0,
    Ch2,
    Ch3,
    Ch4,
    Ch5,
    Ch6,
    Ch7,
    Ch8,
    Ch9,
    Ch10,
    Ch11,
    Ch12,
    Ch13,
    Ch14,
    Ch15,
    Ch16,
}

impl Channel {
    pub const ALL: [Channel; 16] = [
        Channel::Ch1,
        Channel::Ch2,
        Channel::Ch3,
        Channel::Ch4,
        Channel::Ch5,
        Channel::Ch6,
        Channel::Ch7,
        Channel::Ch8,
        Channel::Ch9,
        Channel::Ch10,
        Channel::Ch11,
        Channel::Ch12,
        Channel::Ch13,
        Channel::Ch14,
        Channel::Ch15,
        Channel::Ch16,
    ];

    pub const fn index(self) -> u8 {
        self as u8
    }

    pub const fn number(self) -> u8 {
        self as u8 + 1
    }

    pub fn from_index(index: u8) -> Result<Self, ValueError> {
        Self::ALL
            .get(index as usize)
            .copied()
            .ok_or(ValueError::ChannelIndex(index))
    }

    pub fn from_number(number: u8) -> Result<Self, ValueError> {
        number
            .checked_sub(1)
            .and_then(|index| Self::from_index(index).ok())
            .ok_or(ValueError::ChannelNumber(number))
    }
}

pub(crate) fn channel_from_nibble(status: u8) -> Channel {
    Channel::from_index(status & 0x0F).expect("status nibble is always 0..=15")
}

impl std::fmt::Display for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.number())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_from_number_roundtrips() {
        for ch in Channel::ALL {
            assert_eq!(Channel::from_number(ch.number()), Ok(ch));
        }
        assert_eq!(Channel::from_number(1), Ok(Channel::Ch1));
        assert_eq!(Channel::from_number(16), Ok(Channel::Ch16));
    }

    #[test]
    fn channel_from_number_rejects_out_of_range() {
        assert_eq!(Channel::from_number(0), Err(ValueError::ChannelNumber(0)));
        assert_eq!(Channel::from_number(17), Err(ValueError::ChannelNumber(17)));
        assert_eq!(Channel::from_index(16), Err(ValueError::ChannelIndex(16)));
    }
}
