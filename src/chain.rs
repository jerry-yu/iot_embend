use crate::handler::{H256,Address};
use cita_tool::{
    encode,decode,
    client::{remove_0x,TransactionOptions,
        basic::{ClientExt,Client},
    },
    protos::blockchain::{Transaction, UnverifiedTransaction},
    rpctypes::{JsonRpcResponse,ResponseValue,ParamsValue},
};
use std::collections::BTreeMap;
use std::cell::RefCell;

#[derive(Clone)]
pub enum ChainInfo {
    SucHash(Vec<u8>),
    Height(usize),
    UnsignHash(u16,Vec<u8>),

}

#[derive(Clone)]
pub enum ToChainInfo {
    Data(usize,u64,Vec<u8>),
    Url(String),
    DstAccount(Address),
    Sign(u16,Vec<u8>),
}


#[derive(Default,Clone)]
pub struct ChainOp {
    pub dst_account: Option<Address>,
    pub url: Option<String>,
    pub dir:String,
    pub saved_info:Vec<ToChainInfo>,
    pub saved_tx:BTreeMap<u16,Transaction>,
    pub saved_hash:Vec<H256>,
    pub inc_id: u16,
    
}

impl ChainOp {
    pub fn new(dir:&str)->Self {
        let mut op = ChainOp::default();
        op.dir = dir.to_string();
        op
    }

    pub fn need_config(&self)->bool {
        if self.dst_account.is_none() || self.url.is_none() {
            return true;
        }
        false
    }

    pub fn save_chain_info(&mut self,info:ToChainInfo) {
        self.saved_info.push(info);
    }

    pub fn save_tx(&mut self,idx:u16,tx:Transaction) {
        self.saved_tx.insert(idx,tx);
    }

    pub fn get_id(&mut self) -> u16 {
        let id = self.inc_id;
        self.inc_id = id.wrapping_add(1);
        id
    }

    pub fn file_name(&self,fname:&str) -> String {
        let fname = self.dir.clone() + "/" + fname;
        fname
    }

    pub fn parse_json_hash(res : JsonRpcResponse) -> String {
        if let Some(res) =  res.result() {
            match res {
                ResponseValue::Map(m) => {
                    if let Some(hash) =  m.get("hash") {
                        match hash {
                            ParamsValue::String(hstr) => {
                                return hstr.clone();
                            }
                            _ =>{}
                        }
                    }
                },
                _  => {},
            }
        }
        String::new()
    }

    pub fn parse_json_height(res : JsonRpcResponse) -> usize {
        if let Some(res) =  res.result() {
            match res {
                ResponseValue::Singe(v) => {
                    match v {
                        ParamsValue::String(mut s) => {
                            let s = remove_0x(&s);
                            if let Ok(h) = s.parse::<usize>() {
                                return h;
                            }
                        }
                        _=>{}
                    }
                },
                _  => {},
            }
        }
        0
    }
}