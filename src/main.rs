#![recursion_limit = "2048"]

mod chain;
mod config;
mod handler;
mod payload;

use async_std::fs;
use async_std::future;
use async_std::io::BufReader;
use async_std::net::{TcpListener, TcpStream};
use async_std::prelude::*;
use async_std::sync::{Arc, Mutex};
use async_std::task;
use cita_tool::{
    client::{
        basic::{Client, ClientExt},
        remove_0x, TransactionOptions,
    },
    encode, encode_params,
    protos::blockchain::UnverifiedTransaction,
    ProtoMessage, H256,
};

use chain::{ChainInfo, ChainOp, RawChainData, ToChainInfo, TABLE_SQL};
use cita_tool::{sign, Encryption, KeyPair, PrivateKey};
use clap::{App, Arg};
use config::{AllConfig, Config, DevAddr};
use futures::{channel::mpsc, select, FutureExt, SinkExt};
use handler::{sm3, Address, Iot, PK};
use log::{debug, error, info, trace, warn};
use payload::{
    ChipCommand, Header, Payload, TYPE_CHIP_REQ, TYPE_CHIP_RES, TYPE_ERR_NOTIFY, TYPE_RAW_DATA,
    TYPE_RAW_DATA_RES,
};
use rand::Rng;
use rusqlite::{Connection, NO_PARAMS};
use std::convert::TryInto;
use std::str::FromStr;
use std::time::{Duration, Instant};

const CHAIN_VIST_INTERVAL: u64 = 1;
const CHAIN_HEIGHT_TIMES: u64 = 30;

//const CONF_DIR: &'static str = "./conf";
const FUNC_HASH: &'static str = "fc934fed";

// lazy_static::lazy_static!{
//     static ref ONE_IOT:Arc<Mutex<Iot>> = Arc::new(Mutex::new(Iot::new()));
// }
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

async fn network_process(
    id: usize,
    net_reader: TcpStream,
    mut net_to_main: mpsc::UnboundedSender<(usize, Header, Vec<u8>)>,
) -> Result<()> {
    let mut reader = BufReader::new(net_reader.clone());
    let _err = loop {
        let mut head_buff = vec![0; 10];
        let res = reader.read_exact(&mut head_buff).await;
        if res.is_err() {
            break res;
        }
        if !Header::is_flag_ok(&head_buff) {
            warn!("Header's leader byte not 0x55AA");
            continue;
        }
        let header = Header::parse(&head_buff[2..]);
        let mut body = vec![0; header.len as usize];
        let res = reader.read_exact(&mut body).await;
        if res.is_err() {
            break res;
        }
        trace!("net read data header {:?},body {:x?}", header, body);
        net_to_main.send((id, header, body)).await?;
        trace!("net read data send ok");
    };
    net_to_main.send((id, Header::default(), vec![])).await?;
    Ok(())
}

fn get_pk_from_file(dfile: String) -> Option<Vec<u8>> {
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
    config: AllConfig,
) -> Result<()> {
    let mut iot = Iot::new(&config.dev_file);

    let pk = get_pk_from_file(config.dev_file);
    if let Some(pk) = pk {
        iot.self_pk = Some(pk.clone());
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

    let sql_con = Connection::open("db")?;
    sql_con.execute(TABLE_SQL, NO_PARAMS)?;
    let sql_con = Arc::new(Mutex::new(sql_con));

    if let Ok(to_be_sent_data) = ChainOp::get_unhashed_data(sql_con.clone()).await {
        for data in to_be_sent_data {
            main_to_chain.send(ToChainInfo::Data(data)).await?;
        }
    }

    if let Ok(to_be_del_hash) = ChainOp::get_undeleted_hash(sql_con.clone()).await {
        for hash in to_be_del_hash {
            main_to_chain.send(ToChainInfo::UndecideHash(hash)).await?;
        }
    }

    loop {
        select! {
            info = main_from_chain.next().fuse() => match info {
                Some(info) => {
                    match info {
                        ChainInfo::UnsignHash(req_id,hash) => {
                            let sid = iot.tobe_signed_datas.front().unwrap_or(&(0,vec!())).0;
                            if !iot.need_chip_pk() {
                                // Tobe signed id would never be 0
                                if sid == 0 || sid == req_id  {
                                    let data = Payload::pack_chip_data(ChipCommand::Signature, Some(hash.clone()));
                                    let mut buf = Payload::pack_head_data(TYPE_CHIP_REQ, req_id,data.len() as u32);
                                    buf.extend(data);
                                    debug!("send tobe sig hash req id {}",req_id);
                                    let _ = iot.send_any_net_data(&buf).await;
                                }
                            }
                            if sid != req_id {
                                iot.tobe_signed_datas.push_back((req_id,hash));
                            }
                        }
                        ChainInfo::SignedHash(nonce,hash) => {
                            let _ = ChainOp::update_data_hash(sql_con.clone(),nonce,&hash).await;
                        }
                        ChainInfo::SuccHash(hash) => {
                            let _ = ChainOp::delete_data_with_hash(sql_con.clone(),&hash).await;
                        }
                        _ => {}
                    }
                },
                None => break,
            },
            data = main_from_net.next().fuse() => match data {
                Some((stream_id,header,mut payload)) => {
                    if header.ptype == TYPE_RAW_DATA {
                        if header.len > 8 {
                            let nonce:u64 = rand::thread_rng().gen();
                            let value = u64::from_be_bytes(payload[0..8].try_into().unwrap());
                            let str_data = String::from_utf8_lossy(&payload[8..].to_vec()).into_owned();
                            let res = ChainOp::insert_raw_data(sql_con.clone(),nonce,value,&str_data).await;
                            if let Ok(_) = res {
                                let buf = Payload::pack_head_data(TYPE_RAW_DATA_RES,header.id,0);
                                let res = iot.send_net_data(stream_id, &buf).await;
                                if let Ok(_) =res {
                                    info!("main recv sid {} header {:?} data {:x?}",stream_id,header,str_data);
                                main_to_chain.send(ToChainInfo::Data(RawChainData {
                                    nonce,
                                    value:value,
                                    data:str_data})).await?;
                                } else {
                                    info!("main stream id {} not ok",stream_id);
                                }
                            } else {
                                error!("inert data error {:?}",res);
                            }
                        } else {
                            warn!("recv raw data len {:?}",header.len);
                        }
                    } else if header.ptype == TYPE_CHIP_RES {
                        let len = payload.len();
                        if len == 0x02 {
                            if Payload::is_chip_ok(payload[0], payload[1]) {
                                let data = {
                                    if header.id > 0 {Payload::pack_chip_data(ChipCommand::GetSignature,None)}
                                    else {Payload::pack_chip_data(ChipCommand::GetPK,None)}
                                };
                                let mut hbuf = Payload::pack_head_data(TYPE_CHIP_REQ, header.id, data.len() as u32);
                                hbuf.extend(data);
                                let _ = iot.send_net_data(stream_id, &hbuf).await;
                            } else {
                                error!("chip return error respone {:x}{:x}, expect 0x6140",payload[0], payload[1]);
                            }
                        } else if len == 0x42  {
                            if Payload::is_status_ok(payload[0x40],payload[0x41]) {
                               payload.resize(0x40,0);
                                if header.id > 0 {
                                    iot.remove_signed_data(header.id);
                                    main_to_chain.send(ToChainInfo::Sign(header.id,payload)).await?;
                                    let _ = iot.send_sign_data(stream_id).await;
                                } else {
                                    main_to_chain.send(ToChainInfo::SelfPK(payload.clone())).await?;
                                    iot.self_pk = Some(payload.clone());
                                    //send to be signed data
                                    if let Some((req_id,data)) = iot.tobe_signed_datas.front() {
                                        let data = Payload::pack_chip_data(ChipCommand::Signature, Some(data.clone()));
                                        let mut buf = Payload::pack_head_data(TYPE_CHIP_REQ, *req_id,data.len() as u32);
                                        buf.extend(data);
                                        info!("send tobe sig hash req id {},when pk gotten",req_id);
                                        let _ = iot.send_any_net_data(&buf).await;
                                    }
                                    let hash = sm3::hash::Sm3Hash::new(&payload).get_hash();
                                    // let mut hash: [u8]= [0;32];
                                    // payload.sm3_crypt_hash_into(&mut hash);
                                    let addr = Address::from(H256::from(hash));
                                    let dev = DevAddr {
                                        dev_addr: format!("0x{}",encode(addr)),
                                        dev_pk: format!("0x{}",encode(payload)),
                                    };
                                    let dev_data = toml::to_vec(&dev).unwrap();
                                    let mut fs =  fs::OpenOptions::new()
                                        .write(true)
                                        .create(true)
                                        .truncate(true)
                                        .open(iot.dev_file.clone())
                                        .await?;
                                    fs.write_all(&dev_data).await?;
                                    let _ = iot.send_sign_data(stream_id).await;
                                }
                            } else {
                                error!("chip return error status {:x}{:x} expect 0x9000",payload[0x40], payload[0x41]);
                            }
                        } else {
                            warn!("Unkown payload len {} data {:#?}",len,payload);
                        }

                    } else if header.ptype ==TYPE_ERR_NOTIFY{
                        iot.remove_stream_by_id(stream_id);
                    } else {
                        warn!("recive unkown header type {:?}",header);
                    }
                },
                None=> break,
            },
            tcp = main_from_listener.next().fuse() => match tcp {
                Some((stream_id,tcp)) => {
                    let _ = iot.links.insert(stream_id, tcp);
                    let flag = iot.need_chip_pk();
                    trace!("Incoming stream id {} need pk {}",stream_id,flag);
                    //trace!("saved linkes: {:?}", iot.links);
                    if flag {
                        let data = Payload::pack_chip_data(ChipCommand::CreateKeyPair,None);
                        let mut hbuf = Payload::pack_head_data(TYPE_CHIP_REQ, 0, data.len() as u32);
                        hbuf.extend(data);
                        let _ =iot.send_net_data(stream_id, &hbuf).await;
                    } else {
                        let _ = iot.send_sign_data(stream_id).await;
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
    let mut op = ChainOp::new();
    loop {
        match future::timeout(tval, chain_from_main.next()).await {
            Ok(Some(data)) => {
                count = 0;
                match data {
                    ToChainInfo::Data(raw_data) => {
                        let code_str = encode_params(
                            &["string".to_string(), "uint".to_string()],
                            &[raw_data.data, raw_data.value.to_string()],
                            true,
                        );

                        info!("get raw data id {:?},", raw_data.value);

                        if let Ok(code_str) = code_str {
                            let account_str = format!("{:x}", op.dst_account.unwrap());
                            let nonce_str = format!("{:x}", raw_data.nonce);
                            let mut code = FUNC_HASH.to_string();
                            code = code + &code_str;

                            let mut h = None;
                            if op.chain_height.is_some()
                                && Instant::now().duration_since(op.chain_height.unwrap().0)
                                    < Duration::new(200, 0)
                            {
                                h = Some(op.chain_height.unwrap().1)
                            } else {
                                let res = hc.lock().await.get_block_number();
                                info!("chain_loop get block number {:?}", res);
                                if let Ok(res) = res {
                                    if let Some(qh) = ChainOp::parse_json_height(res) {
                                        op.chain_height = Some((Instant::now(), qh));
                                        h = Some(qh);
                                    }
                                } else {
                                    warn!("chain_loop can't get block number");
                                    continue;
                                }
                            }

                            let tx_opt = TransactionOptions::new()
                                .set_current_height(h)
                                .set_code(&code)
                                .set_address(&account_str);
                            info!(
                                "encode_params  account {:?} nonce {:?}",
                                account_str, nonce_str
                            );
                            let res = hc.lock().await.generate_transaction(tx_opt);
                            if let Ok(mut tx) = res {
                                // Use input random number as nonce
                                // info!(
                                //     "chain raw data gen nonce {:?} data {:x?}",
                                //     nonce_str,
                                //     tx.get_data()
                                // );
                                debug!("generate_transaction ok");
                                tx.set_nonce(nonce_str);
                                if op.tx_empty() {
                                    if let Some(hash) = op.chain_to_sign_data(&tx) {
                                        let inc_id = op.get_id();
                                        op.save_tx(inc_id, tx);
                                        chain_to_main
                                            .send(ChainInfo::UnsignHash(inc_id, hash))
                                            .await?;
                                    }
                                } else {
                                    let id = op.get_id();
                                    op.save_tx(id, tx);
                                }
                            } else {
                                warn!("generate transaction error {:?}", res);
                            }
                        } else {
                            warn!("encode param error");
                        }
                    }
                    ToChainInfo::SelfPK(pk) => {
                        op.self_pk = Some(pk);
                    }
                    ToChainInfo::Sign(id, sign_data) => {
                        if let Some(tx) = op.remove_tx(id) {
                            if let Some((id, tx)) = op.first_tx() {
                                if let Some(buf) = op.chain_to_sign_data(&tx) {
                                    chain_to_main.send(ChainInfo::UnsignHash(id, buf)).await?;
                                }
                            }
                            let nstr = tx.get_nonce();
                            let nonce = u64::from_str_radix(nstr, 16).unwrap();
                            trace!("get nonce {:?} data {:x?}", nonce, tx.get_data());
                            let mut utx = UnverifiedTransaction::new();
                            utx.set_transaction(tx.clone());
                            let mut sign_data = sign_data.clone();
                            sign_data.extend(op.self_pk.clone().unwrap());
                            utx.set_signature(sign_data);

                            if let Ok(bytes_code) = utx.write_to_bytes() {
                                let utx_str = encode(bytes_code);

                                if let Ok(res) = hc.lock().await.send_signed_transaction(&utx_str) {
                                    info!("sent tx get res {:?}", res);
                                    let hash = ChainOp::parse_json_hash(res);
                                    if !hash.is_empty() {
                                        // TODO: Send hash should before doing send_signed_transaction
                                        op.checked_hashes.push_back(hash.clone());
                                        chain_to_main
                                            .send(ChainInfo::SignedHash(
                                                nonce,
                                                remove_0x(&hash).to_string(),
                                            ))
                                            .await?;
                                        //fs::rename(op.file_name(nonce),op.file_name(&hash)).await;
                                    }
                                }
                            }
                        } else {
                            info!("not find signed id {}", id);
                        }
                    }
                    // timestamp should add,and check
                    ToChainInfo::UndecideHash(hash) => {
                        op.checked_hashes.push_back(hash);
                    }
                    ToChainInfo::Uri(uri) => {
                        info!("chain_loop uri {}", uri);
                        let tmp_hc = hc.lock().await.clone();
                        *hc.lock().await = tmp_hc.set_uri(&uri);
                    }
                    ToChainInfo::DstAccount(addr) => {
                        info!("chain_loop dst  {}", addr);
                        op.dst_account = Some(addr);
                    } //_ => {}
                }
            }
            Ok(None) => {}
            Err(_) => {
                debug!(
                    "chain_loop timeout  count {} tiem  {:?}",
                    count,
                    Instant::now(),
                );
                // If not Recieve message from main,resend message
                if count % CHAIN_HEIGHT_TIMES == CHAIN_HEIGHT_TIMES - 1 {
                    if let Some((id, tx)) = op.first_tx() {
                        if let Some(buf) = op.chain_to_sign_data(&tx) {
                            chain_to_main.send(ChainInfo::UnsignHash(id, buf)).await?;
                        }
                    }
                }

                if let Some(hash) = op.checked_hashes.pop_front() {
                    if let Ok(res) = hc.lock().await.get_transaction(&hash) {
                        if res.result().is_none() {
                            op.checked_hashes.push_back(hash);
                        } else {
                            chain_to_main.send(ChainInfo::SuccHash(hash)).await?;
                        }
                    }
                }
                count += 1;
            }
        }
    }
}

async fn send_onchain_info(mut stream: TcpStream, limit: usize) -> Result<()> {
    let mut flag: usize = 0;
    loop {
        std::thread::sleep(std::time::Duration::new(15, 0));
        let data_str = format!(
            "{{
            \"cardNo\":\"FF{}\",
            \"parkingTime\":\"20分\",
            \"parkingFee\":{}
            }}",
            flag, flag
        );

        info!("client send to chain: {}", data_str);
        let mut buff = flag.to_be_bytes().to_vec();
        buff.extend(data_str.as_bytes());

        let mut header = Header::default();
        header.ptype = TYPE_RAW_DATA;
        header.len = buff.len() as u32;
        let buff = Payload::pack_head_and_payload(header, buff);
        stream.write_all(&buff).await?;
        flag += 1;
        if flag >= limit && limit != 0 {
            break;
        }
    }
    Ok(())
}

async fn client_start(config: AllConfig) -> Result<()> {
    let sk = PrivateKey::from_str(
        "6ff0a6e8cd3b19cfc17503a8cdf4b7fc5aafe63ab81749a32b7fb5555eb8771f",
        Encryption::from_str("sm2").unwrap(),
    )
    .unwrap();
    let key_pair = KeyPair::from_privkey(sk);

    let mut stream = TcpStream::connect("127.0.0.1:18888").await?;
    let mut reader = BufReader::new(stream.clone());
    let mut sent_info: Vec<u8> = Vec::new();

    task::spawn(send_onchain_info(stream.clone(), config.client_send_limit));

    loop {
        let mut head_buff = vec![0; 10];
        reader.read_exact(&mut head_buff).await?;
        if !Header::is_flag_ok(&head_buff) {
            warn!("client get header falg not ok");
            continue;
        }

        let header = Header::parse(&head_buff[2..]);
        info!("client get header {:?} ", header);
        let mut body = vec![0; header.len as usize];
        reader.read_exact(&mut body).await?;
        if header.ptype == TYPE_CHIP_REQ {
            let key_create: Vec<u8> = vec![0x80, 0x45, 0x00, 0x00, 0x00];
            let get_data: Vec<u8> = vec![0x00, 0xC0, 0x00, 0x00, 0x40];
            let tobe_sign: Vec<u8> = vec![0x80, 0x46, 0x00, 0x00, 0x20];
            if &body[0..5] == &key_create[0..] {
                let data = Payload::pack_chip_data(ChipCommand::ChipReady, None);
                let mut ack_header = header;
                ack_header.ptype = TYPE_CHIP_RES;
                ack_header.len = data.len() as u32;
                let buf = Payload::pack_head_and_payload(ack_header, data);
                stream.write_all(&buf).await?;
                sent_info = key_pair.pubkey().to_vec();
                sent_info.push(0x90);
                sent_info.push(0x00);
            } else if &body[0..5] == &tobe_sign[0..] {
                let data = Payload::pack_chip_data(ChipCommand::ChipReady, None);
                let mut ack_header = header;
                ack_header.ptype = TYPE_CHIP_RES;
                ack_header.len = data.len() as u32;
                let buf = Payload::pack_head_and_payload(ack_header, data);
                stream.write_all(&buf).await?;
                let hash = H256::from(&body[5..]);
                let sig = sign(&key_pair.privkey(), &hash);
                sent_info = sig.to_vec()[0..64].to_vec();
                sent_info.push(0x90);
                sent_info.push(0x00);
            } else if &body[0..5] == &get_data[0..] {
                let mut ack_header = header;
                ack_header.ptype = TYPE_CHIP_RES;
                ack_header.len = sent_info.clone().len() as u32;
                let buf = Payload::pack_head_and_payload(ack_header, sent_info.clone());
                stream.write_all(&buf).await?;
            }
        }
    }
}

async fn server_start(all_config: AllConfig) -> Result<()> {
    let listener = TcpListener::bind("0.0.0.0:18888").await?;
    info!("Listening on {}", listener.local_addr()?);

    let (main_to_chain, chain_from_main) = mpsc::unbounded();
    let (chain_to_main, main_from_chain) = mpsc::unbounded();
    let (net_to_main, main_from_net) = mpsc::unbounded();
    let (mut listener_to_main, main_from_listener) = mpsc::unbounded();

    let ptsk = task::spawn(main_process(
        main_to_chain,
        main_from_chain,
        main_from_net,
        main_from_listener,
        all_config,
    ));

    let ctsk = task::spawn(chain_loop(chain_to_main.clone(), chain_from_main));

    let mut incoming = listener.incoming();
    let mut stream_id: usize = 0;
    while let Some(stream) = incoming.next().await {
        let stream = stream?;
        listener_to_main.send((stream_id, stream.clone())).await?;
        task::spawn(network_process(stream_id, stream, net_to_main.clone()));
        stream_id += 1;
    }
    let _ = ptsk.await;
    let _ = ctsk.await;
    Ok(())
}

fn main() -> Result<()> {
    env_logger::init();

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
                //info!("file current {:?} file {:?}",std::fs::canonicalize("."),conf_file);
                let fc = std::fs::read_to_string(conf_file);
                if fc.is_err() {
                    std::thread::sleep(std::time::Duration::new(2, 0));
                    continue;
                }
                // info!("file content fc {:?}",fc);
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
        task::block_on(client_start(all_config))
    }
}
