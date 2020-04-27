#![recursion_limit = "1024"]

mod chain;
mod config;
mod handler;
mod payload;

use async_std::fs;
use async_std::future;
use async_std::io::{self, BufReader};
use async_std::net::{SocketAddr, TcpListener, TcpStream};
use async_std::prelude::*;
use async_std::sync::{Arc, Mutex};
use async_std::task;
use cita_tool::{
    client::{
        basic::{Client, ClientExt},
        remove_0x, TransactionOptions,
    },
    decode, encode, encode_params,
    protos::blockchain::{Transaction, UnverifiedTransaction},
    rpctypes::{JsonRpcResponse, ParamsValue, ResponseValue},
    ProtoMessage, H256, U256,
};

use chain::{ChainInfo, ChainOp, RawChainData, ToChainInfo};
use cita_tool::{
    pubkey_to_address, secp256k1_sign, sign, sm2_sign, CreateKey, Encryption, Hashable, KeyPair,
    Message, PrivateKey, PubKey, Secp256k1KeyPair, Secp256k1PrivKey, Secp256k1PubKey, Signature,
    Sm2KeyPair, Sm2Privkey, Sm2Pubkey, Sm2Signature,
};
use clap::{App, Arg};
use config::{AllConfig, Config, DevAddr};
use futures::{channel::mpsc, select, FutureExt, SinkExt};
use handler::{sm3, Address, Iot, PK};
use payload::{
    ChipCommand, Header, Payload, TYPE_CHIP_REQ, TYPE_CHIP_RES, TYPE_RAW_DATA, TYPE_RAW_DATA_RES,
};
use rand::Rng;
use sqlite::Connection;
use std::convert::TryInto;
use std::str::FromStr;
use std::time::Duration;

const CHAIN_VIST_INTERVAL: u64 = 3;
const CHAIN_HEIGHT_TIMES: u64 = 9;
const CHAIN_TX_HASH_TIMES: u64 = 3;

const WAL_DIR: &'static str = "./wal";
const CONF_DIR: &'static str = "./conf";
const FUNC_HASH: &'static str = "2c0e9055";
const TABLE_SQL: &'static str = " 
CREATE TABLE IF NOT EXISTS txs(
    id INTEGER PRIMARY KEY, 
    value INTEGER NOT NULL,
    data TEXT NOT NULL,
    hash BLOB default NULL);";

// lazy_static::lazy_static!{
//     static ref ONE_IOT:Arc<Mutex<Iot>> = Arc::new(Mutex::new(Iot::new()));
// }
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

async fn network_process(
    id: usize,
    net_reader: TcpStream,
    mut net_to_main: mpsc::UnboundedSender<(usize, Header, Vec<u8>)>,
) -> io::Result<()> {
    let mut reader = BufReader::new(net_reader.clone());
    loop {
        println!("network");
        let mut head_buff = vec![0; 8];
        reader.read_exact(&mut head_buff).await?;
        println!("read_exact");
        let header = Header::parse(&head_buff);
        let mut body = vec![0; header.len as usize];
        reader.read_exact(&mut body).await?;
        net_to_main.send((id, header, body)).await;
    }
}

fn get_pk_from_file(dfile:String) ->Option<Vec<u8>> {
    if let Ok(dv) = std::fs::read_to_string(dfile) {
        if let Ok(dev) = toml::from_str::<DevAddr>(&dv) {
            if !dev.dev_addr.is_empty() && !dev.dev_pk.is_empty() {
                if let Ok(pk) = PK::from_str(remove_0x(&dev.dev_pk)) {
                   return Some(pk.to_vec());
                }
            }
        }
    }
    None
    
}


async fn main_process(
    mut main_to_chain: mpsc::UnboundedSender<ToChainInfo>,
    mut main_from_chain: mpsc::UnboundedReceiver<ChainInfo>,
    mut main_from_net: mpsc::UnboundedReceiver<(usize, Header, Vec<u8>)>,
    mut main_from_listener: mpsc::UnboundedReceiver<(usize, TcpStream)>,
    sql_con: Connection,
    config: AllConfig,
) -> Result<()> {
    let mut iot = Iot::new("", &config.dev_file);
    let pk = get_pk_from_file(config.dev_file);
    if let Some(pk) = pk {
        iot.self_pk = Some(pk);
        main_to_chain.send(ToChainInfo::SelfPK(pk)).await?;
    }
    
    let addr = Address::from_str(remove_0x(&config.conf.account));
    if addr.is_err() {
        std::process::exit(1);
    }
    main_to_chain
        .send(ToChainInfo::Uri(config.conf.url.clone()))
        .await?;
    main_to_chain
        .send(ToChainInfo::DstAccount(addr.unwrap()))
        .await?;

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
                            let _ = iot.send_any_net_data(&buf).await;
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

                        let str_data = String::from_utf8(payload[8..].to_vec());
                        if let Ok(str_data) = str_data {
                            let res = ChainOp::insert_raw_data(&sql_con,nonce,value,&str_data);
                            if let Ok(_) = res {
                                let buf = Payload::pack_head_data(TYPE_RAW_DATA_RES,header.id,0);
                                iot.send_net_data(stream_id, &buf).await;
                                println!("main recv sid {} header {:?} data {:?}",stream_id,header,str_data);
                                main_to_chain.send(ToChainInfo::Data(RawChainData {
                                    nonce,
                                    value:value,
                                    data:str_data})).await?;

                            } else {
                                // println!("inert data error {:?}",res.error());
                            }
                        }
                    } else if header.ptype == TYPE_CHIP_RES {
                        if payload.len() == 0x02 && Payload::is_chip_ok(payload[0], payload[1]) {
                            let data = {
                                if header.id > 0 {Payload::pack_chip_data(ChipCommand::GetSignature,None)}
                                else {Payload::pack_chip_data(ChipCommand::GetPK,None)}
                            };
                            let mut hbuf = Payload::pack_head_data(TYPE_CHIP_REQ, header.id, data.len() as u32);
                            hbuf.extend(data);
                            iot.send_net_data(stream_id, &hbuf).await;
                        } else if payload.len() == 0x40 {
                            if header.id > 0 {
                                main_to_chain.send(ToChainInfo::Sign(header.id,payload)).await?;
                            } else {
                                main_to_chain.send(ToChainInfo::SelfPK(payload.clone())).await?;
                                let hash = sm3::hash::Sm3Hash::new(&payload).get_hash();
                                let addr = Address::from(H256::from(hash));

                                let dev = DevAddr {
                                    dev_addr: addr.to_string(),
                                    dev_pk: encode(payload),
                                };

                                let dev_data = toml::to_vec(&dev).unwrap();
                                let mut fs =  fs::OpenOptions::new()
                                    .write(true)
                                    .create(true)
                                    .truncate(true)
                                    .open(iot.dev_file.clone())
                                    .await?;
                                fs.write_all(&dev_data).await?;
                            }
                        }

                    } else {
                        println!("recive unkown header type {:?}",header);
                    }
                },
                None=> break,
            },
            tcp = main_from_listener.next().fuse() => match tcp {
                Some((stream_id,tcp)) => {
                    let _ = iot.links.insert(stream_id, tcp);
                    if iot.need_chip_pk() {
                        let data =Payload::pack_chip_data(ChipCommand::CreateKeyPair,None);
                        let mut hbuf = Payload::pack_head_data(TYPE_CHIP_REQ, 0, data.len() as u32);
                        hbuf.extend(data);
                        iot.send_net_data(stream_id, &hbuf).await;
                    }
                },
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
    let tval = Duration::new(CHAIN_VIST_INTERVAL, 0);
    let hc = Mutex::new(Client::new());
    let mut count: u64 = 0;
    let mut op = ChainOp::new(WAL_DIR);
    loop {
        match future::timeout(tval, chain_from_main.next()).await {
            Ok(Some(data)) => {
                match data {
                    ToChainInfo::Data(raw_data) => {
                        let tx_opt = TransactionOptions::new();
                        let code_str = encode_params(
                            &["string".to_string(), "uint".to_string()],
                            &[raw_data.data, raw_data.value.to_string()],
                            true,
                        );
                        println!("encode_params {:?}", code_str);
                        if let Ok(code_str) = code_str {
                            let account_str = format!("{:x}", op.dst_account.unwrap());
                            let nonce_str = format!("{:x}", raw_data.nonce);
                            let mut code = FUNC_HASH.to_string();
                            code = code + &code_str;
                            tx_opt.set_code(&code);
                            tx_opt.set_address(&account_str);
                            println!(
                                "encode_params code {:?},account {:?} nonce {:?}",
                                code, account_str, nonce_str
                            );
                            if let Ok(mut tx) = hc.lock().await.generate_transaction(tx_opt) {
                                // Use input random number as nonce
                                tx.set_nonce(nonce_str);
                                if let Ok(btx) = tx.write_to_bytes() {
                                    let hash = sm3::hash::Sm3Hash::new(&btx).get_hash();
                                    let inc_id = op.get_id();
                                    println!("tx sm3 hash {:?} inc_id {}", hash, inc_id);
                                    op.save_tx(inc_id, tx);

                                    chain_to_main
                                        .send(ChainInfo::UnsignHash(inc_id, hash.to_vec()))
                                        .await?;
                                } else {
                                    println!("Tx write error");
                                }
                            } else {
                                //op.save_chain_info(ToChainInfo::Data(raw_data));
                            }
                        } else {
                        }
                    }
                    ToChainInfo::SelfPK(pk) => {
                        op.self_pk = Some(pk);
                    }

                    ToChainInfo::Sign(id, sign_data) => {
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
                                        chain_to_main
                                            .send(ChainInfo::SignedHash(
                                                nonce,
                                                decode(hash).unwrap(),
                                            ))
                                            .await?;
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
                        println!("chain_loop uri {}", uri);
                        let tmp_hc = hc.lock().await.clone();
                        *hc.lock().await = tmp_hc.set_uri(&uri);
                    }
                    ToChainInfo::DstAccount(addr) => {
                        println!("chain_loop dst  {}", addr);
                        op.dst_account = Some(addr);
                    }
                    _ => {}
                }
            }
            Ok(None) => {}
            Err(_) => {
                // println!("chain_loop timeout");
                if count % CHAIN_HEIGHT_TIMES == 0 {
                    if let Ok(res) = hc.lock().await.get_block_number() {
                        println!("chain loop get something {:?}", res);
                        let h = ChainOp::parse_json_height(res);
                        if h > 0 {
                            chain_to_main.send(ChainInfo::Height(h)).await?;
                        }
                    }
                }

                count += 1;
            }
        }
    }
}

async fn client_start(all_config: AllConfig) -> Result<()> {
    let key_pair = KeyPair::new(Encryption::from_str("sm2").unwrap());
    let stream = TcpStream::connect("127.0.0.1:18888").await?;
  
    let mut reader = BufReader::new(stream.clone());
    let mut flag:usize = 0;
    loop {
        println!("network client");
        let mut head_buff = vec![0; 8];
        reader.read_exact(&mut head_buff).await?;
        println!("client read_exact");
        let header = Header::parse(&head_buff);
        let mut body = vec![0; header.len as usize];
        reader.read_exact(&mut body).await?;

        if header.ptype == TYPE_CHIP_REQ {
            let key_create:Vec<u8> = vec![0x80, 0x45, 0x00, 0x00, 0x00];
            let get_data:Vec<u8> = vec![0x00, 0xC0, 0x00, 0x00, 0x40];
            let sign:Vec<u8> = vec![0x80, 0x46, 0x00, 0x00, 0x20];
            if &body[0..5] == &key_create[0..] {
                flag = 1;
                
            } else if &body[0..5] == &get_data[0..] {

            } if &body[0..5] == &sign[0..] {

            }
        }
        
    }
    
    

    Ok(())
}

async fn server_start(all_config: AllConfig) -> Result<()> {
    let listener = TcpListener::bind("0.0.0.0:18888").await?;
    println!("Listening on {}", listener.local_addr()?);

    let (main_to_chain, chain_from_main) = mpsc::unbounded();
    let (chain_to_main, main_from_chain) = mpsc::unbounded();
    let (net_to_main, main_from_net) = mpsc::unbounded();
    let (mut listener_to_main, main_from_listener) = mpsc::unbounded();
    let connection = sqlite::open("db").unwrap();
    connection.execute(TABLE_SQL).unwrap();

    let ctsk = task::spawn(chain_loop(chain_to_main.clone(), chain_from_main));
    let ptsk = task::spawn(main_process(
        main_to_chain,
        main_from_chain,
        main_from_net,
        main_from_listener,
        connection,
        all_config,
    ));

    let mut incoming = listener.incoming();
    let mut stream_id: usize = 0;
    while let Some(stream) = incoming.next().await {
        let stream = stream?;
        listener_to_main.send((stream_id, stream.clone())).await;
        task::spawn(network_process(stream_id, stream, net_to_main.clone()));
        stream_id += 1;
    }
    let _ = ptsk.await;
    let _ = ctsk.await;
    Ok(())
}

fn main() -> Result<()> {
    let all_config = {
        let mut all_config = AllConfig::default();
        let matches = App::new("IoT")
            .version("1.0")
            .author("Rivtower")
            .about("CITA Block Chain IoT powered by Rust")
            .arg(
                Arg::with_name("config")
                    .short("c")
                    .long("config")
                    .value_name("FILE")
                    .help("Sets a custom config file")
                    .takes_value(true),
            )
            .arg(
                Arg::with_name("devconf")
                    .short("d")
                    .long("devconf")
                    .value_name("FILE")
                    .help("Sets a dev config file")
                    .takes_value(true),
            )
            .arg(
                Arg::with_name("t")
                    .short("t")
                    .multiple(false)
                    .help("Set start as client"),
            )
            .get_matches();

        let conf_file = matches.value_of("config").unwrap_or("chain_config.toml");
        let dev_config = matches.value_of("devconf").unwrap_or("dev_addr.toml");
        match matches.occurrences_of("t") {
            0 => all_config.client_start = false,
            _ => all_config.client_start = true,
        }
        all_config.dev_file = dev_config.to_string();
        if !all_config.client_start {
            let conf = loop {
                //println!("file current {:?} file {:?}",std::fs::canonicalize("."),conf_file);
                let fc = std::fs::read_to_string(conf_file);
                if fc.is_err() {
                    std::thread::sleep(std::time::Duration::new(2, 0));
                    continue;
                }
                // println!("file content fc {:?}",fc);
                if let Ok(config) = toml::from_str::<Config>(&fc.unwrap()) {
                    if !config.url.is_empty() && !config.account.is_empty() {
                        break config;
                    }
                }
                std::thread::sleep(std::time::Duration::new(2, 0));
            };
            all_config.conf = conf;
        }
        all_config
    };

    if !all_config.client_start {
        task::block_on(server_start(all_config))
    } else {
        Ok(())
    }
}
