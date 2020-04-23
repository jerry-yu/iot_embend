use std::convert::TryInto;

#[derive(Copy,Clone,Debug,Default)]
pub struct Header {
    pub len: u32,
    pub id: u16,
    pub ptype: u8,
    pub reserve: u8,
}

impl Header {
    pub fn parse(data: &[u8]) -> Self {
        assert_eq!(data.len(), 8);
        Header {
            len: u32::from_be_bytes(data[0..4].try_into().unwrap()),
            id: u16::from_be_bytes(data[4..6].try_into().unwrap()),
            ptype: data[6],
            reserve: data[7],
        }
    }

    pub fn to_vec(&self) -> Vec<u8> {
        let mut data = Vec::new();
        data.copy_from_slice(&self.len.to_be_bytes());
        data.extend_from_slice(&self.id.to_be_bytes());
        data.push(self.ptype);
        data.push(self.reserve);
        data
    }
}
