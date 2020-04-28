use std::convert::TryInto;

pub const TYPE_CHIP_REQ: u8 = 1;
pub const TYPE_CHIP_RES: u8 = 2;
pub const TYPE_RAW_DATA: u8 = 3;
pub const TYPE_RAW_DATA_RES: u8 = 3;

pub enum ChipCommand {
    Select,
    CreateKeyPair,
    GetPK,
    Signature,
    GetSignature,
    ChipReady,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct Payload {}

impl Payload {
    pub fn pack_chip_data(cmd: ChipCommand, in_data: Option<Vec<u8>>) -> Vec<u8> {
        match cmd {
            ChipCommand::CreateKeyPair => vec![0x80, 0x45, 0x00, 0x00, 0x00],
            ChipCommand::GetPK | ChipCommand::GetSignature => vec![0x00, 0xC0, 0x00, 0x00, 0x40],
            ChipCommand::Select => vec![
                0x00, 0xA4, 0x04, 0x00, 0x06, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66,
            ],
            ChipCommand::Signature => {
                let mut v: Vec<u8> = vec![0x80, 0x46, 0x00, 0x00, 0x20];
                if in_data.is_none() {
                    return Vec::new();
                }
                v.extend(in_data.unwrap());
                v
            }
            ChipCommand::ChipReady => vec![0x61, 0x40],
            _ => vec![],
        }
    }

    pub fn pack_head_and_payload(hd: Header, data: Vec<u8>) -> Vec<u8> {
        let mut buf = hd.to_vec();
        buf.extend(data);
        buf
    }

    pub fn pack_head_data(ptype: u8, id: u16, len: u32) -> Vec<u8> {
        let mut req_head = Header::default();
        req_head.id = id;
        req_head.ptype = ptype;
        req_head.len = len;
        req_head.to_vec()
    }

    pub fn is_chip_ok(fb: u8, sb: u8) -> bool {
        fb == 0x61 && sb == 0x40
    }
}

#[derive(Copy, Clone, Debug, Default)]
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
        data.extend_from_slice(&self.len.to_be_bytes());
        data.extend_from_slice(&self.id.to_be_bytes());
        data.push(self.ptype);
        data.push(self.reserve);
        data
    }
}
