#![recursion_limit="1024"]

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
    encode,decode,ProtoMessage,
    client::{remove_0x,TransactionOptions,
        basic::{ClientExt,Client},
    },
    protos::blockchain::{Transaction, UnverifiedTransaction},
    rpctypes::{JsonRpcResponse,ResponseValue,ParamsValue},
};
use ethereum_types::{H256,U256};
use futures::{
    channel::{mpsc},
    select, FutureExt, SinkExt,
};
use handler::{Address,Iot,sm3,};
use chain::{ToChainInfo,ChainInfo ,ChainOp,RawChainData};
use payload::{Header,TYPE_RAW_DATA,TYPE_CHIP_REQ,TYPE_CHIP_RES,TYPE_RAW_DATA_RES,ChipCommand,Payload};
use std::time::Duration;
use rand::Rng;
use std::convert::TryInto;
use sqlite::Connection;

const CHAIN_VIST_INTERVAL:u64 = 3;
const CHAIN_HEIGHT_TIMES:u64 = 2;
const CHAIN_TX_HASH_TIMES:u64 = 3;

const WAL_DIR: &'static str = "./wal";
const CONF_DIR: &'static str = "./conf";

//const FUNC_HASH:Vec<u8> = vec!(0xff,0xff,0xff);

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
    mut main_to_chain: mpsc::UnboundedSender<ToChainInfo>,
    mut main_from_chain:  mpsc::UnboundedReceiver<ChainInfo>,
    mut main_from_net:  mpsc::UnboundedReceiver<(usize,Header,Vec<u8>)>,
    mut main_from_listener : mpsc::UnboundedReceiver<(usize,TcpStream)>,
    sql_con : Connection,
) -> Result<()> {
    let mut iot = Iot::new(WAL_DIR,CONF_DIR);

    let to_be_sent_data = ChainOp::get_unhashed_data(&sql_con);
    for data in to_be_sent_data {
        main_to_chain.send(ToChainInfo::Data(data)).await?;
    }

    let to_be_del_hash = ChainOp::get_undeleted_hash(&sql_con);
    for hash in to_be_del_hash {
        main_to_chain.send(ToChainInfo::DelHash(hash)).await?;
    }
    
    loop {
        select! {
            info = main_from_chain.next().fuse() => match info {
                Some(info) => {
                    match info {
                        ChainInfo::UnsignHash(req_id,data) => {
                            let data = Payload::pack_chip_data(ChipCommand::Signature, Some(data));
                            let mut buf = Payload::pack_head_data(TYPE_CHIP_REQ, req_id,data.len() as u32);
                            buf.extend(data);
                            if let Ok(stream_id) = iot.send_any_net_data(&buf).await {
                                iot.change_state(stream_id, true);
                            }
                        }

                        ChainInfo::SignedHash(nonce,hash) => {
                            ChainOp::update_data_hash(&sql_con,nonce,&hash);
                        }

                        _ => {}
                    }
                },
                None => break,
            },
            data = main_from_net.next().fuse() => match data {
                Some((stream_id,header,payload)) => {
                    if header.ptype == TYPE_RAW_DATA {
                        let nonce:u64 = rand::thread_rng().gen();
                        let value = u64::from_be_bytes(payload[0..8].try_into().unwrap());
                        let data = &payload[8..] ;
                        let res = ChainOp::insert_raw_data(&sql_con,nonce,value,data);
                        if let Ok(_) = res {
                            let buf = Payload::pack_head_data(TYPE_RAW_DATA_RES,header.id,0);
                            iot.send_net_data(stream_id, &buf).await;

                            main_to_chain.send(ToChainInfo::Data(RawChainData {
                                nonce,
                                value:value,
                                data:data.to_vec(),})).await?;
                            
                        } else {
                            // println!("inert data error {:?}",res.error());
                        }
                        
                    } else if header.ptype == TYPE_CHIP_RES {
                        if let Some(state) = iot.get_state(stream_id) {
                            if payload.len() == 0x02 && Payload::is_chip_ok(payload[0], payload[1]) {
                                let data = {
                                    if state {Payload::pack_chip_data(ChipCommand::GetSignature,None)}
                                    else {Payload::pack_chip_data(ChipCommand::GetPK,None)}
                                };
                                let mut hbuf = Payload::pack_head_data(TYPE_CHIP_REQ, header.id, data.len() as u32);
                                hbuf.extend(data);
                                iot.send_net_data(stream_id, &hbuf).await;
                            } else if payload.len() == 0x40 {
                                if state {
                                    main_to_chain.send(ToChainInfo::Sign(header.id,payload)).await?;
                                } else {
                                    //TODO: save pubkey
                                }
                            }
                        } else {
                            println!("Not has sent request");
                        }
                    } else {
                        println!("recive unkown header type {:?}",header);
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

async fn chain_loop(
    mut chain_to_main: mpsc::UnboundedSender<ChainInfo>,
    mut chain_from_main: mpsc::UnboundedReceiver<ToChainInfo>,
) -> Result<()> {
    let tval = Duration::new(CHAIN_VIST_INTERVAL,0);
    let hc = Client::new().set_uri("http://122.112.142.180:1337");
    let hc = Mutex::new(hc);
    let mut count:u64 = 0;
    let mut op =  ChainOp::new(WAL_DIR);
    loop {
        match future::timeout(tval,chain_from_main.next()).await {
            Ok(Some(data)) => {
                match data {
                    ToChainInfo::Data(raw_data) => {
                        let tx_opt = TransactionOptions::new();
                        let code_str = format!("{}",encode(raw_data.data));
                        let account_str = format!("{:x}",op.dst_account.unwrap());
                        let nonce_str = format!("{:x}",raw_data.nonce);
                        tx_opt.set_code(&code_str);
                        tx_opt.set_value(Some(raw_data.value.into()));
                        tx_opt.set_address(&account_str);

                        if let Ok(mut tx) = hc.lock().await.generate_transaction(tx_opt) {
                            tx.set_nonce(nonce_str);
                            if let Ok(btx) = tx.write_to_bytes() {
                                let hash = sm3::hash::Sm3Hash::new(&btx).get_hash();
                                let id = op.get_id();
                                op.save_tx(id,tx);

                                chain_to_main.send(ChainInfo::UnsignHash(id,hash.to_vec())).await?;
                            } else {
                                println!("Tx write error");
                            }
                            
                        } else {
                            //op.save_chain_info(ToChainInfo::Data(raw_data));
                        }
                    }

                    ToChainInfo::SelfPK(pk) => {
                        op.self_pk = Some(pk);
                    }
            
                    ToChainInfo::Sign(id,sign_data) => {
                        let mut utx = UnverifiedTransaction::new();
                        if let Some(tx) = op.saved_tx.get(&id) {
                            let nonce = tx.get_nonce().parse::<u64>().unwrap();
                            utx.set_transaction(tx.clone());
                            let mut sign_data = sign_data.clone();
                            sign_data.extend(op.self_pk.clone().unwrap());
                            utx.set_signature(sign_data);

                            if let Ok(bytes_code) = utx.write_to_bytes() {
                                let utx_str = encode(bytes_code);
                                if let Ok(res) = hc.lock().await.send_signed_transaction(&utx_str) {
                                    let hash = ChainOp::parse_json_hash(res);
                                    if !hash.is_empty() {
                                        // TODO: Send hash should before doing send_signed_transaction
                                        chain_to_main.send(ChainInfo::SignedHash(nonce,decode(hash).unwrap())).await?;
                                        //fs::rename(op.file_name(nonce),op.file_name(&hash)).await;
                                    }
                                }
                            }
                        }
                        op.saved_tx.remove(&id);
                    }
                    // timestamp should add,and check
                    ToChainInfo::DelHash(hash) => {
                        op.checked_hashes.push(hash);
                    }
                    ToChainInfo::Uri(uri) => {
                        let tmp_hc = hc.lock().await.clone();
                        *hc.lock().await = tmp_hc.set_uri(&uri);
                    }
                    ToChainInfo::DstAccount(addr) => {
                       op.dst_account = Some(addr);
                    }
                    _ =>{}
                }
            },
            Ok(None) => {}
            Err(_) => {
                // println!("chain_loop timeout");
                if count % CHAIN_HEIGHT_TIMES == 0 {
                    if let Ok(res) = hc.lock().await.get_block_number() {
                        println!("chain loop get something {:?}",res);
                        let h = ChainOp::parse_json_height(res); 
                        if h > 0 {
                            chain_to_main.send(ChainInfo::Height(h)).await?;
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
    
    
    task::block_on(async {
        let listener = TcpListener::bind("0.0.0.0:18888").await?;
        println!("Listening on {}", listener.local_addr()?);

        let (main_to_chain,  chain_from_main) = mpsc::unbounded();
        let (chain_to_main,  main_from_chain) = mpsc::unbounded();
        let (net_to_main,  main_from_net) = mpsc::unbounded();
        let (mut listener_to_main,  main_from_listener) = mpsc::unbounded();
        let connection = sqlite::open("./db").unwrap();
        connection
        .execute("
            CREATE TABLE IF NOT EXISTS txs(
                id INTEGER PRIMARY KEY, 
                value INTEGER NOT NULL,
                data TEXT NOT NULL,
                hash TEXT default NULL);
        ",)
        .unwrap();

        let ctsk = task::spawn(chain_loop(chain_to_main.clone(), chain_from_main));
        let ptsk = task::spawn(main_process(main_to_chain, main_from_chain, main_from_net,main_from_listener,connection));

        let mut incoming = listener.incoming();
        let mut stream_id:usize = 0;
        while let Some(stream) = incoming.next().await {
           let stream = stream?;
           listener_to_main.send((stream_id,stream.clone())).await;
           task::spawn(network_process(stream_id,stream, net_to_main.clone()));
           stream_id+=1;
        }
         let _ = ptsk.await;
         let _ =ctsk.await;
        Ok(())
    })
}
