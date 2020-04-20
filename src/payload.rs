use std::convert::TryInto;

pub struct Header {
    pub len: u32,
    pub id: u16,
    pub ptype: u8,
    _reserve: u8,
}

impl Header {
    pub fn parse(data: &[u8]) -> Self {
        assert_eq!(data.len(), 8);
        Header {
            len: u32::from_be_bytes(data[0..4].try_into().unwrap()),
            id: u16::from_be_bytes(data[4..6].try_into().unwrap()),
            ptype: data[6],
            _reserve: data[7],
        }
    }
}
