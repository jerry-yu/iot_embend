use crate::handler::{sm3, Address, H256};
use crate::Result;
use cita_tool::{
    client::{
        basic::{Client, ClientExt},
        remove_0x, TransactionOptions,
    },
    decode, encode,
    protos::blockchain::{Transaction, UnverifiedTransaction},
    rpctypes::{JsonRpcResponse, ParamsValue, ResponseValue},
    ProtoMessage,
};
use sqlite::{Connection, Statement};
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

pub const TABLE_SQL: &'static str = " 
CREATE TABLE IF NOT EXISTS txs(
    id INTEGER PRIMARY KEY, 
    value INTEGER NOT NULL,
    data TEXT NOT NULL,
    hash TEXT default NULL);";

#[derive(Clone)]
pub struct RawChainData {
    pub nonce: u64,
    pub value: u64,
    pub data: String,
}

#[derive(Clone)]
pub enum ChainInfo {
    SuccHash(String),
    Height(usize),
    UnsignHash(u16, Vec<u8>),
    SignedHash(u64, String),
}

#[derive(Clone)]
pub enum ToChainInfo {
    Data(RawChainData),
    UndecideHash(String),
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
    pub inc_id: u16,
    pub checked_hashes: VecDeque<String>,
    pub self_pk: Option<Vec<u8>>,
    pub chain_height: Option<(Instant, u64)>,
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

    pub fn get_undeleted_hash(sql_con: &Connection) -> Vec<String> {
        let mut cursor = sql_con
            .prepare("SELECT hash FROM txs WHERE hash is not null")
            .unwrap()
            .cursor();

        //cursor.bind(&[Value::Integer(50)]).unwrap();
        let mut out = Vec::new();
        while let Some(row) = cursor.next().unwrap() {
            out.push(row[0].as_string().unwrap().to_string());
        }
        out
    }

    pub fn insert_raw_data(sql_con: &Connection, id: u64, value: u64, data: &str) -> Result<()> {
        let mut state = sql_con
            .prepare("insert into txs(id,value,data,hash) values(?,?,?,null)")
            .unwrap();
        println!("insert db id {}",id);
        state.bind(1, id as i64).unwrap();
        state.bind(2, value as i64).unwrap();
        state.bind(3, data).unwrap();
        let _ = state.next()?;
        Ok(())
    }

    pub fn update_data_hash(sql_con: &Connection, id: u64, hash: &str) -> Result<()> {
        let mut state = sql_con
            .prepare("update txs set hash = ? where id = ? and hash is null")
            .unwrap();
        println!("update db id {} hash {}",id,hash);
        state.bind(1, hash).unwrap();
        state.bind(2, id as i64).unwrap();
        let _ = state.next()?;
        Ok(())
    }

    pub fn delete_data_with_hash(sql_con: &Connection, hash: &str) -> Result<()> {
        let mut state = sql_con.prepare("delete from txs where hash = ?").unwrap();
        println!("delete db hash {}",hash);
        state.bind(1, hash).unwrap();
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

    pub fn first_tx(&self) -> Option<(u16, Transaction)> {
        if let Some((id, tx)) = self.saved_tx.iter().next() {
            return Some((id.clone(), tx.clone()));
        }
        None
    }

    pub fn remove_tx(&mut self, idx: u16) -> Option<Transaction> {
        self.saved_tx.remove(&idx)
    }

    pub fn tx_empty(&self) -> bool {
        self.saved_tx.is_empty()
    }

    pub fn chain_to_sign_data(&mut self, tx: &Transaction) -> Option<Vec<u8>> {
        if let Ok(btx) = tx.write_to_bytes() {
            let hash = sm3::hash::Sm3Hash::new(&btx).get_hash();
            return Some(hash.to_vec());
        }
        None
    }

    pub fn get_id(&mut self) -> u16 {
        self.inc_id = self.inc_id.wrapping_add(1);
        if self.inc_id == 0 {
            self.inc_id = 1;
        }
        self.inc_id
    }

    pub fn parse_json_hash(res: JsonRpcResponse) -> String {
        if let Some(res) = res.result() {
            match res {
                ResponseValue::Map(m) => {
                    if let Some(hash) = m.get("hash") {
                        match hash {
                            ParamsValue::String(hstr) => {
                                return remove_0x(hstr).to_string();
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

    pub fn parse_json_height(res: JsonRpcResponse) -> Option<u64> {
        if let Some(res) = res.result() {
            match res {
                ResponseValue::Singe(v) => match v {
                    ParamsValue::String(s) => {
                        let s = remove_0x(&s);
                        return u64::from_str_radix(s, 16).ok();
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        None
    }
}
