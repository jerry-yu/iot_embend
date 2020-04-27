use crate::handler::{Address, H256};
use crate::Result;
use cita_tool::{
    client::{
        basic::{Client, ClientExt},
        remove_0x, TransactionOptions,
    },
    decode, encode,
    protos::blockchain::{Transaction, UnverifiedTransaction},
    rpctypes::{JsonRpcResponse, ParamsValue, ResponseValue},
};
use sqlite::{Connection, Statement};
use std::cell::RefCell;
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct RawChainData {
    pub nonce: u64,
    pub value: u64,
    pub data: String,
}

#[derive(Clone)]
pub enum ChainInfo {
    SuccHash(Vec<u8>),
    Height(usize),
    UnsignHash(u16, Vec<u8>),
    SignedHash(u64, Vec<u8>),
}

#[derive(Clone)]
pub enum ToChainInfo {
    Data(RawChainData),
    DelHash(H256),
    Uri(String),
    DstAccount(Address),
    SelfPK(Vec<u8>),
    Sign(u16, Vec<u8>),
}

#[derive(Default, Clone)]
pub struct ChainOp {
    pub dst_account: Option<Address>,
    pub url: Option<String>,
    pub dir: String,
    pub saved_info: Vec<ToChainInfo>,
    pub saved_tx: BTreeMap<u16, Transaction>,
    pub saved_hash: Vec<H256>,
    pub inc_id: u16,
    pub checked_hashes: Vec<H256>,
    pub self_pk: Option<Vec<u8>>,
}

impl ChainOp {
    pub fn new(dir: &str) -> Self {
        let mut op = ChainOp::default();
        op.dir = dir.to_string();
        op.inc_id = 1;
        op
    }

    pub fn get_unhashed_data(sql_con: &Connection) -> Vec<RawChainData> {
        let mut cursor = sql_con
            .prepare("SELECT id,value,data FROM txs WHERE hash is null")
            .unwrap()
            .cursor();

        let mut out = Vec::new();
        while let Some(row) = cursor.next().unwrap() {
            out.push(RawChainData {
                nonce: row[0].as_integer().unwrap() as u64,
                value: row[1].as_integer().unwrap() as u64,
                data: row[2].as_string().unwrap().to_string(),
            });
        }
        out
    }

    pub fn get_undeleted_hash(sql_con: &Connection) -> Vec<H256> {
        let mut cursor = sql_con
            .prepare("SELECT hash FROM txs WHERE hash is not null")
            .unwrap()
            .cursor();

        //cursor.bind(&[Value::Integer(50)]).unwrap();
        let mut out = Vec::new();
        while let Some(row) = cursor.next().unwrap() {
            out.push(H256::from_slice(row[0].as_binary().unwrap()));
        }
        out
    }

    pub fn insert_raw_data(sql_con: &Connection, id: u64, value: u64, data: &str) -> Result<()> {
        let mut state = sql_con
            .prepare("insert into txs(id,value,data,hash) values(?,?,?,null)")
            .unwrap();

        state.bind(1, id as i64).unwrap();
        state.bind(2, value as i64).unwrap();
        state.bind(3, data).unwrap();
        let _ = state.next()?;
        Ok(())
    }

    pub fn update_data_hash(sql_con: &Connection, id: u64, hash: &[u8]) -> Result<()> {
        let mut state = sql_con
            .prepare("update txs set hash = ? where id = ? and hash is null")
            .unwrap();

        state.bind(1, hash).unwrap();
        state.bind(2, id as i64).unwrap();
        let _ = state.next()?;
        Ok(())
    }

    pub fn need_config(&self) -> bool {
        if self.dst_account.is_none() || self.url.is_none() {
            return true;
        }
        false
    }

    pub fn save_chain_info(&mut self, info: ToChainInfo) {
        self.saved_info.push(info);
    }

    pub fn save_tx(&mut self, idx: u16, tx: Transaction) {
        self.saved_tx.insert(idx, tx);
    }

    pub fn get_id(&mut self) -> u16 {
        let id = self.inc_id;
        self.inc_id = id.wrapping_add(1);
        if self.inc_id == 0 {
            self.inc_id = 1;
        }
        id
    }

    pub fn parse_json_hash(res: JsonRpcResponse) -> String {
        if let Some(res) = res.result() {
            match res {
                ResponseValue::Map(m) => {
                    if let Some(hash) = m.get("hash") {
                        match hash {
                            ParamsValue::String(hstr) => {
                                return hstr.clone();
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        String::new()
    }

    pub fn parse_json_height(res: JsonRpcResponse) -> usize {
        if let Some(res) = res.result() {
            match res {
                ResponseValue::Singe(v) => match v {
                    ParamsValue::String(mut s) => {
                        let s = remove_0x(&s);
                        if let Ok(h) = s.parse::<usize>() {
                            return h;
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        0
    }
}
