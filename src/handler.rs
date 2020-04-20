use async_std::prelude::*;
use async_std::task;
use cita_tool::{
    client::basic::Client,
    protos::blockchain::{Transaction, UnverifiedTransaction},
};
use ethereum_types::{H160, H256};
use futures::{channel::mpsc, select, FutureExt, SinkExt};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use async_std::net::TcpStream;

pub type Sender<T> = mpsc::UnboundedSender<T>;
pub type Receiver<T> = mpsc::UnboundedReceiver<T>;
pub type Address = H160;

const TYPE_HB: u8 = 0;
const TYPE_PK_REQ: u8 = 1;
const TYPE_PK_RES: u8 = 2;
const TYPE_SIGN_REQ: u8 = 3;
const TYPE_SIGN_RES: u8 = 4;
const TYPE_URL_REQ: u8 = 7;
const TYPE_URL_RES: u8 = 8;
const TYPE_ACCOUNT_REQ: u8 = 9;
const TYPE_ACCOUNT_RES: u8 = 10;
const TYPE_TOBE_SENT_DATA: u8 = 11;

pub enum ChainInfo {
    UnverifiedTransaction,
    Sender,
}

#[derive(Clone)]
pub struct Iot {
    pub rpc_url: Option<String>,
    pub dst_account: Option<Address>,
    pub my_account: Option<Address>,
    pub used: bool,
    pub tcp: Option<TcpStream>,
    //pub to_chain : Sender<ChainInfo>,
    //pub from_chain :Receiver<H256>,
}

impl Iot {
    pub fn new() -> Self {
        Iot {
            rpc_url: None,
            dst_account: None,
            my_account: None,
            used: false,
            tcp:None,
            //to_chain,
        }
    }

    pub fn set_stream(&mut self,tcp:TcpStream) {
        self.tcp = Some(tcp);
    }

    pub fn proc_body(&mut self, ptype: u8, body: &[u8]) -> bool {
        match ptype {
            TYPE_HB => {}
            TYPE_PK_RES => {}
            _ => {}
        }

        true
    }

    // pub async fn send(self,buf: &[u8]) -> async_std::io::Result<()> {
    //     self.tcp.and_then(|tcp| async { tcp.send(buf).await?}).or()
    // }
}
