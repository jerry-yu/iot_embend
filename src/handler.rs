use async_std::prelude::*;
use async_std::task;
use cita_tool::{
    client::basic::Client,
    protos::blockchain::{Transaction, UnverifiedTransaction},
};
pub use ethereum_types::{H160, H256};
use futures::{channel::mpsc, select, FutureExt, SinkExt};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use async_std::net::{TcpStream};
use std::collections::BTreeMap;
pub use libsm::sm3;
use std::convert::TryInto;
use crate::payload::Header;
use std::io::Read;
use std::str::FromStr;
pub type Address = H160;


const URL_FILE :&'static str = "url";
const ACCOUNT_FILE :&'static str = "dst_account";
const SELF_ACCOUNT_FILE :&'static str = "my_account";

#[derive(Clone)]
pub struct Iot {
    pub rpc_url: Option<String>,
    pub dst_account: Option<Address>,
    pub my_account: Option<Address>,
    pub used: bool,
    pub links: BTreeMap<usize,TcpStream>,
    pub states: BTreeMap<usize,bool>,
}

impl Iot {
    pub fn new(dir:&str,conf_dir:&str) -> Self {
        std::fs::DirBuilder::new()
        .recursive(true)
        .create(dir).unwrap();

        let mut tmp = Iot {
            rpc_url: None,
            dst_account: None,
            my_account: None,
            used: false,
            links:BTreeMap::new(),
            states:BTreeMap::new(),
        };
        tmp.load_file(dir,conf_dir);
        tmp
    }

    pub fn get_state(&self,stream_id:usize) ->Option<bool> {
        self.states.get(&stream_id).cloned()
    }

    // false: wait for pubkey ,true wait for signature
    pub fn change_state(&mut self,stream_id:usize,state:bool) {
        self.states.insert(stream_id,state);
    }

    pub fn clean_state(&mut self,stream_id:usize) {
        self.states.remove(&stream_id);
    }

    pub fn get_tcp(&self,stream_id:usize) ->Option<&TcpStream> {
        self.links.get(&stream_id)
    }

    fn load_file(&mut self,dir:&str,conf_dir:&str) {
        let url_file = conf_dir.to_string() + "/"+ URL_FILE;
        let dst_account_file = conf_dir.to_string()+"/"+ACCOUNT_FILE;
        let my_account_file = conf_dir.to_string()+"/"+SELF_ACCOUNT_FILE; 
        if let Ok(mut fs) = std::fs::File::open(url_file) {
            let mut buf=String::new();
            if let Ok(_) = fs.read_to_string(&mut buf) {
                self.rpc_url = Some(buf);
            }
        }
        if let Ok(mut fs) = std::fs::File::open(dst_account_file) {
            let mut buf=String::new();
            if let Ok(_) = fs.read_to_string(&mut buf) {
                self.dst_account = Address::from_str(&buf).ok();
            }
        }
        if let Ok(mut fs) = std::fs::File::open(my_account_file) {
            let mut buf=String::new();
            if let Ok(_) = fs.read_to_string(&mut buf) {
                self.my_account = Address::from_str(&buf).ok();
            }
        }
    }

    pub fn need_config(&self) ->bool {
        if self.rpc_url.is_none() ||  self.dst_account.is_none() ||  self.my_account.is_none() {
            return true;
        }
        false
    }

    pub async fn send_config_req(&mut self,id:usize) ->std::io::Result<()> {
        let mut header = Header::default();
        header.id = id as u16;
        header.len = 0;
        Ok(())
    }

    pub async fn send_net_data(&mut self,id:usize,data: &[u8]) ->std::io::Result<()>  {
        if let Some(tcp) = self.links.get_mut(&id) {
            tcp.write_all(data).await?;
        }
        Ok(())
    }

    pub async fn send_any_net_data(&mut self,data: &[u8]) -> std::io::Result<usize> {
        let mut res = Ok(());
        for (idx,tcp) in &mut self.links {
          res = tcp.write_all(data).await;
          if res.is_ok() {
              return Ok(*idx);
          }
        }
        Err(res.unwrap_err())
    }

    pub fn proc_body(&mut self, id:usize, hder: Header, body: &[u8]) -> Option<Vec<u8>> {
        println!("iot proc body headerf {:?}",hder);
        // match hder.ptype {
        //     TYPE_PK_RES => {
        //         let hash = sm3::hash::Sm3Hash::new(body).get_hash();
        //         self.my_account = Some(Address::from_slice(&hash));

        //         {
        //             let mut ack_head = Header::default();
        //             // just use this;non sense
        //             ack_head.id = id as u16;
        //             ack_head.ptype = 0xff;
        //             let mut abytes = self.my_account.unwrap().as_bytes().to_vec();
        //             ack_head.len = abytes.len() as u32;
        //             let mut data = ack_head.to_vec();
        //             data.append(&mut abytes);
        //             return Some(data);
        //         }
        //     }
        //     TYPE_OP_PK_REQ => {
        //         let mut ack_head = hder;
        //         ack_head.ptype = 0xff;

        //         if self.my_account.is_some() {
        //             let mut abytes = self.my_account.unwrap().as_bytes().to_vec();
        //             ack_head.len = abytes.len() as u32;
        //             let mut data = ack_head.to_vec();
        //             data.append(&mut abytes);
        //             return Some(data);
        //         } else {
        //             ack_head.len = 0;
        //             return Some(ack_head.to_vec());
        //         }
        //     }
            

        //     _ => {
        //         println!("get data unused type, header {:?}",hder);
        //     }
        // }

        None
        //Some("hello".as_bytes().into())
    }
}
