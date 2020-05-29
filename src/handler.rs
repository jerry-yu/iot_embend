use async_std::prelude::*;
use async_std::task;
use cita_tool::{
    client::basic::Client,
    protos::blockchain::{Transaction, UnverifiedTransaction},
    H160, H512,
};
use log::{info, trace};

pub use cita_tool::H256;
//pub use ethereum_types::{};
use crate::payload::{ChipCommand, Header, Payload, TYPE_CHIP_REQ};
use async_std::net::TcpStream;
use futures::{channel::mpsc, select, FutureExt, SinkExt};
pub use libsm::sm3;
use std::collections::{BTreeMap, VecDeque};
use std::convert::TryInto;
use std::io::Read;
use std::str::FromStr;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub type Address = H160;
pub type PK = H512;

// const URL_FILE: &'static str = "url";
// const ACCOUNT_FILE: &'static str = "dst_account";
// const SELF_ACCOUNT_FILE: &'static str = "my_account";

#[derive(Clone)]
pub struct Iot {
    pub self_pk: Option<Vec<u8>>,
    pub dev_file: String,
    pub used: bool,
    pub links: BTreeMap<usize, TcpStream>,
    pub tobe_signed_datas: VecDeque<(u16, Vec<u8>)>,
    //pub states: BTreeMap<usize, bool>,
}

impl Iot {
    pub fn new(dev_file: &str) -> Self {
        // std::fs::DirBuilder::new()
        //     .recursive(true)
        //     .create(dir)
        //     .unwrap();

        Iot {
            self_pk: None,
            dev_file: dev_file.to_string(),
            used: false,
            links: BTreeMap::new(),
            tobe_signed_datas: VecDeque::new(),
        }
    }

    // pub fn get_tobe_signed(&self, stream_id: usize) -> Option<bool> {
    //     self.states.get(&stream_id).cloned()
    // }

    // // false: wait for pubkey ,true wait for signature
    // pub fn change_state(&mut self, stream_id: usize, state: bool) {
    //     self.states.insert(stream_id, state);
    // }

    // pub fn clean_state(&mut self, stream_id: usize) {
    //     self.states.remove(&stream_id);
    // }
    pub fn get_first_tobe_signed_data(&self) -> Option<(u16, Vec<u8>)> {
        self.tobe_signed_datas.front().cloned()
    }

    pub fn remove_signed_data(&mut self, req_id: u16) {
        if let Some(data) = self.tobe_signed_datas.pop_front() {
            if data.0 == req_id {
                return;
            } else {
                info!("first signed data not equal reqid {}", req_id);
            }
        }
    }

    pub fn need_chip_pk(&self) -> bool {
        self.self_pk.is_none()
    }

    pub async fn send_net_data(&mut self, id: usize, data: &[u8]) -> std::io::Result<()> {
        if let Some(tcp) = self.links.get_mut(&id) {
            trace!("send net data single idx:{} data: {:x?}", id, data);
            tcp.write_all(data).await?;
        }
        Ok(())
    }

    pub async fn send_sign_data(&mut self) -> std::io::Result<()> {
        if let Some((req_id, hash)) = self.get_first_tobe_signed_data() {
            let data = Payload::pack_chip_data(ChipCommand::Signature, Some(hash));
            let mut buf = Payload::pack_head_data(TYPE_CHIP_REQ, req_id, data.len() as u32);
            buf.extend(data);
            trace!("send tobe sig hash req id {}", req_id);
            self.send_any_net_data(&buf).await?;
        }
        Ok(())
    }

    pub async fn remove_stream_by_id(&mut self, stream_id: usize) {
        self.links.remove(&stream_id);
    }

    pub async fn send_any_net_data(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let mut res = Err(std::io::ErrorKind::NotFound.into());
        for (idx, tcp) in &mut self.links {
            trace!("send net data idx:{} data: {:x?}", idx, data);
            res = tcp.write_all(data).await;
            if res.is_ok() {
                return Ok(*idx);
            }
        }
        Err(res.unwrap_err())
    }
}
