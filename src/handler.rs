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
use async_std::net::{TcpStream};
use std::collections::HashMap;
use libsm::sm3;
use std::convert::TryInto;
use crate::payload::Header;
use rand::prelude::*;
use std::io::Read;
use std::str::FromStr;

pub type Address = H160;

const TYPE_HB: u8 = 0;
const TYPE_PK_REQ: u8 = 1;
const TYPE_PK_RES: u8 = 2;
const TYPE_SIGN_REQ: u8 = 3;
const TYPE_SIGN_RES: u8 = 4;
const TYPE_OP_PK_REQ: u8 = 5;
const TYPE_OP_PK_RES: u8 = 6;
const TYPE_URL_REQ: u8 = 7;
const TYPE_URL_RES: u8 = 8;
const TYPE_ACCOUNT_REQ: u8 = 9;
const TYPE_ACCOUNT_RES: u8 = 10;
const TYPE_TOBE_SENT_DATA: u8 = 11;
const TYPE_TOBE_ACK_DATA: u8 = 12;

const URL_FILE :&'static str = "url";
const ACCOUNT_FILE :&'static str = "dst_account";
const SELF_ACCOUNT_FILE :&'static str = "my_account";

#[derive(Clone)]
pub struct Iot {
    pub rpc_url: Option<String>,
    pub dst_account: Option<Address>,
    pub my_account: Option<Address>,
    pub used: bool,
    pub links: HashMap<usize,TcpStream>,
    //current_hight: 
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
            links:HashMap::new(),
        };
        tmp.load_file(dir,conf_dir);
        tmp
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

        if self.rpc_url.is_none() {
            header.ptype = TYPE_URL_REQ;
            self.send(id,&header.clone().to_vec()).await?;
        }  
        if self.dst_account.is_none() {
            header.ptype = TYPE_ACCOUNT_REQ;
            self.send(id,&header.clone().to_vec()).await?;
        }
        if self.my_account.is_none() {
            header.ptype = TYPE_PK_REQ;
            self.send(id,&header.clone().to_vec()).await?;
        }
        Ok(())
    }

    pub async fn send(&mut self,id:usize,data: &[u8]) ->std::io::Result<()>  {
        if let Some(tcp) = self.links.get_mut(&id) {
            tcp.write_all(data).await?;
        }
        Ok(())
    }

    pub fn proc_body(&mut self, id:usize, hder: Header, body: &[u8]) -> Option<Vec<u8>> {
        println!("iot proc body");
        let x = 
        let hash = sm3::hash::Sm3Hash::new(body).get_hash();
        match hder.ptype {
            TYPE_HB => {
                
            }
            TYPE_PK_RES => {
                let hash = sm3::hash::Sm3Hash::new(body).get_hash();
                self.my_account = Some(Address::from_slice(&hash));

                {
                    let mut ack_head = Header::default();
                    // just use this;non sense
                    ack_head.id = id as u16;
                    ack_head.ptype = TYPE_OP_PK_RES;
                    let mut abytes = self.my_account.unwrap().as_bytes().to_vec();
                    ack_head.len = abytes.len() as u32;
                    let mut data = ack_head.to_vec();
                    data.append(&mut abytes);
                    return Some(data);
                }
            }
            TYPE_OP_PK_REQ => {
                let mut ack_head = hder;
                ack_head.ptype = TYPE_OP_PK_RES;

                if self.my_account.is_some() {
                    let mut abytes = self.my_account.unwrap().as_bytes().to_vec();
                    ack_head.len = abytes.len() as u32;
                    let mut data = ack_head.to_vec();
                    data.append(&mut abytes);
                    return Some(data);
                } else {
                    ack_head.len = 0;
                    return Some(ack_head.to_vec());
                }
            }
            TYPE_SIGN_RES => {

            }
            TYPE_URL_RES => {
                if let Ok(url) = String::from_utf8(body.to_vec()) {
                    self.rpc_url = Some(url);
                }
            }
            TYPE_ACCOUNT_RES => {
                self.dst_account = Some(Address::from_slice(body));
            }
            TYPE_TOBE_SENT_DATA => {
                let mut ack_head = hder;
                ack_head.ptype = TYPE_TOBE_ACK_DATA;
                ack_head.len = 4;
                let ack_data = vec!(0;4);
                let mut data = ack_head.to_vec();
                data.append(&mut ack_data);
                return Some(data);
            }

            _ => {
                println!("get data unused type, header {:?}",hder);
            }
        }

        None
        //Some("hello".as_bytes().into())
    }
}
