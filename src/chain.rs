use crate::handler::{sm3, Address, H256};
use crate::Result;
use async_std::sync::{Arc, Mutex};
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
use rusqlite::{params, Connection, Statement, NO_PARAMS};
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
    pub saved_info: Vec<ToChainInfo>,
    pub saved_tx: VecDeque<(u16, Transaction)>,
    pub inc_id: u16,
    pub checked_hashes: VecDeque<String>,
    pub self_pk: Option<Vec<u8>>,
    pub chain_height: Option<(Instant, u64)>,
}

impl ChainOp {
    pub fn new() -> Self {
        let mut op = ChainOp::default();
        op.inc_id = 1;
        op
    }

    pub async fn get_unhashed_data(sql_con: Arc<Mutex<Connection>>) -> Result<Vec<RawChainData>> {
        let sql_con = sql_con.lock().await;
        let mut stmt = sql_con.prepare("SELECT id,value,data FROM txs WHERE hash is null")?;
        let mut rows = stmt.query(NO_PARAMS)?;

        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let nonce: i64 = row.get(0)?;
            let value: i64 = row.get(1)?;
            let data = row.get(2)?;

            out.push(RawChainData {
                nonce: nonce as u64,
                value: value as u64,
                data,
            });
        }
        Ok(out)
    }

    pub async fn get_undeleted_hash(sql_con: Arc<Mutex<Connection>>) -> Result<Vec<String>> {
        let sql_con = sql_con.lock().await;
        let mut stmt = sql_con.prepare("SELECT hash FROM txs WHERE hash is not null")?;

        let mut rows = stmt.query(NO_PARAMS)?;

        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(row.get(0)?);
        }
        Ok(out)
    }

    pub async fn insert_raw_data(
        sql_con: Arc<Mutex<Connection>>,
        id: u64,
        value: u64,
        data: &str,
    ) -> Result<()> {
        sql_con.lock().await.execute(
            "insert into txs(id,value,data,hash) values(?,?,?,null)",
            params![id as i64, value as i64, data],
        )?;

        Ok(())
    }

    pub async fn update_data_hash(
        sql_con: Arc<Mutex<Connection>>,
        id: u64,
        hash: &str,
    ) -> Result<()> {
        sql_con.lock().await.execute(
            "update txs set hash = ? where id = ? and hash is null",
            params![hash, id as i64],
        )?;
        Ok(())
    }

    pub async fn delete_data_with_hash(sql_con: Arc<Mutex<Connection>>, hash: &str) -> Result<()> {
        sql_con
            .lock()
            .await
            .execute("delete from txs where hash = ?l", params![hash])?;
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
        self.saved_tx.push_back((idx, tx));
    }

    pub fn first_tx(&self) -> Option<(u16, Transaction)> {
        self.saved_tx.front().cloned()
    }

    pub fn remove_tx(&mut self, idx: u16) -> Option<Transaction> {
        let fir_tx = self.saved_tx.front();
        if fir_tx.is_some() && fir_tx.unwrap().0 == idx {
            return Some(self.saved_tx.pop_front().unwrap().1);
        } else {
            let mut pos = 0;
            for (p, (sid, _)) in self.saved_tx.iter().enumerate() {
                if *sid == idx {
                    pos = p
                }
            }
            if pos > 0 {
                return Some(self.saved_tx.remove(pos).unwrap().1);
            }
            None
        }
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
