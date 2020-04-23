#![recursion_limit="256"]

mod handler;
mod payload;
mod chain;

use async_std::io::{self, BufReader};
use async_std::net::{TcpListener, TcpStream,SocketAddr};
use async_std::prelude::*;
use async_std::sync::{Arc, Mutex};
use async_std::task;
use async_std::future;
use async_std::fs;
use cita_tool::{
    client::{remove_0x,
        basic::{ClientExt,Client},
    },
    protos::blockchain::{Transaction, UnverifiedTransaction},
    rpctypes::{JsonRpcResponse,ResponseValue,ParamsValue},
};
use ethereum_types::{H256};
use futures::{
    channel::{mpsc},
    select, FutureExt, SinkExt,
};
use handler::{Address,Iot};
use chain::ChainOp;
use payload::Header;
use std::time::Duration;

const CHAIN_VIST_INTERVAL:u64 = 3;
const CHAIN_HEIGHT_TIMES:u64 = 2;
const CHAIN_TX_HASH_TIMES:u64 = 3;

const WAL_DIR: &'static str = "./wal";
const CONF_DIR: &'static str = "./conf";

// lazy_static::lazy_static!{
//     static ref ONE_IOT:Arc<Mutex<Iot>> = Arc::new(Mutex::new(Iot::new()));
// }
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

async fn network_process(id:usize,net_reader: TcpStream, mut net_to_main:  mpsc::UnboundedSender<(usize,Header,Vec<u8>)>) -> io::Result<()> {
    let mut reader = BufReader::new(net_reader.clone());
    loop {
        println!("network");
        let mut head_buff = vec![0; 8];
        reader.read_exact(&mut head_buff).await?;
        println!("read_exact");
        let header = Header::parse(&head_buff);
        // let mut body = vec![0; header.len as usize];
        // reader.read_exact(&mut body).await?;

       let mut body = vec!();

        net_to_main.send((id,header,body)).await;
    }

}

async fn main_process(
    main_to_chain: mpsc::UnboundedSender<ToChainInfo>,
    mut main_from_chain:  mpsc::UnboundedReceiver<ChainInfo>,
    mut main_from_net:  mpsc::UnboundedReceiver<(usize,Header,Vec<u8>)>,
    mut main_from_listener : mpsc::UnboundedReceiver<(usize,TcpStream)>,
) -> Result<()> {
    let mut iot = Iot::new(WAL_DIR,CONF_DIR);
    let mut rng = rand::thread_rng();
    if !iot.need_config() {

    }
    loop {
        select! {
            info = main_from_chain.next().fuse() => match info {
                Some(info) => {
                    match info {
                        ChainInfo::Height(h) => {
                            
                        }

                        _ => {}
                    }
                },
                None => break,
            },
            data = main_from_net.next().fuse() => match data {
                Some((id,header,payload)) => {
                    if let Some(data) = iot.proc_body(id,header, &payload) {
                        if heder.ptype == TYPE_TOBE_SENT_DATA {
                            let rnum:u64 = rand::thread_rng().gen();
                            let fname = format!("{}/{:x}",WAL_DIR,rnum);

                            pritnln!("file num {:?}",fname);

                            let mut file = File::create(fname).await?;
                            file.write_all(payload).await?;
                            
                            main_to_chain.send((rnum,payload.to_vec())).await?;
                            //let value:u64 = u64::from_be_bytes(body[0..8].try_into().unwrap());
                        }
                        iot.send(id,&data).await;
                        
                    }
                },
                None=> break,
            },
            tcp = main_from_listener.next().fuse() => match tcp {
                Some((id,tcp)) => {
                    let _ = iot.links.insert(id, tcp);
                    if iot.need_config() {
                        iot.send_config_req(id).await;
                    }
                } ,
                None => break,
                
            },
        }
    }
    
    Ok(())
}

pub enum ChainInfo {
    SucHash(H256),
    Height(usize),
    SentHash(H256),
    
}

pub enum ToChainInfo {
    Data(usize,Vec<u8>),
    Url(String),
    DstAccount(Address),
}

async fn chain_loop(
    mut chain_to_main: mpsc::UnboundedSender<ChainInfo>,
    mut chain_from_main: mpsc::UnboundedReceiver<ToChainInfo>,
) -> Result<()> {
    let tval = Duration::new(CHAIN_VIST_INTERVAL,0);
    let mut hc = Client::new().set_uri("http://122.112.142.180:1337");
    let hc = Mutex::new(hc);
    let mut count:u64 = 0;
    let mut op =  ChainOp::default();
    loop {
        match future::timeout(tval,chain_from_main.next()).await {
            Ok(Some(data)) => {
                match data {
                    ToChainInfo::Data(nonce,data) => {

                    }
                    ToChainInfo::DstAccount(account) => {
                        op.dst_account = Some(account);
                    }
                    ToChainInfo::Url(url) => {
                        hc.lock().await.set_uri(&url);
                    }
                }
                
               
            },
            Ok(None) => {}
            Err(_) => {
                // println!("chain_loop timeout");
                if count % CHAIN_HEIGHT_TIMES == 0 {
                    if let Ok(res) = hc.lock().await.get_block_number() {
                        println!("chain loop get something {:?}",res);
                        if let Some(res) =  res.result() {
                           
                            match res {
                                ResponseValue::Singe(v) => {
                                    match v {
                                        ParamsValue::String(mut s) => {
                                            let s = remove_0x(&s);
                                            if let Ok(h) = s.parse::<usize>() {
                                                println!("chain get height str {:?}",h);
                                                chain_to_main.send(ChainInfo::Height(h)).await?;
                                            }
                                        }
                                        _=>{}
                                    }
                                },
                                _  => {},
                            }
                        }

                    }
                }
             
                count+=1;
            },
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    //let (chain_sender,from_chain) = mpsc::unbounded::<H256>();
    //let iot_static: Arc<Mutex<Iot>> = Arc::new(Mutex::new(Iot::new()));
    
    task::block_on(async {
        let listener = TcpListener::bind("0.0.0.0:18888").await?;
        println!("Listening on {}", listener.local_addr()?);

        let (main_to_chain,  chain_from_main) = mpsc::unbounded();
        let (chain_to_main,  main_from_chain) = mpsc::unbounded();
        let (net_to_main,  main_from_net) = mpsc::unbounded();
        let (mut listener_to_main,  main_from_listener) = mpsc::unbounded();

        let ctsk = task::spawn(chain_loop(chain_to_main.clone(), chain_from_main));

        let ptsk = task::spawn(main_process(main_to_chain, main_from_chain, main_from_net,main_from_listener));

        let mut incoming = listener.incoming();
        let mut stream_id:usize = 0;
        while let Some(stream) = incoming.next().await {
           let stream = stream?;
           listener_to_main.send((stream_id,stream.clone())).await;
           task::spawn(network_process(stream_id,stream, net_to_main.clone()));
           stream_id+=1;
        }
         ptsk.await;
         ctsk.await;
        Ok(())
    })
}
