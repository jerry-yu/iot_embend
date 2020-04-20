mod handler;
mod payload;

use async_std::io::{self, BufReader};
use async_std::net::{TcpListener, TcpStream};
use async_std::prelude::*;
use async_std::sync::{Arc, Mutex};
use async_std::task;
use cita_tool::{
    client::basic::Client,
    protos::blockchain::{Transaction, UnverifiedTransaction},
};
use ethereum_types::{H160, H256};
use futures::{
    channel::{mpsc, oneshot},
    select, FutureExt, SinkExt,
};
use handler::ChainInfo;
use handler::{Iot, Receiver, Sender};
use payload::Header;

lazy_static::lazy_static!{
    static ref ONE_IOT:Arc<Mutex<Iot>> = Arc::new(Mutex::new(Iot::new()));
}

async fn network_process(mut net_reader: Option<TcpStream>) -> io::Result<Option<(Header,Vec<u8>)>> {
    if net_reader.is_none() {
        return Ok(None);
    }
    let net_reader = net_reader.unwrap();
    let mut reader = BufReader::new(net_reader.clone());
    //loop {
        println!("network");
        let mut head_buff = vec![0; 8];
        reader.read_exact(&mut head_buff).await?;
        println!("read_exact");
        let header = Header::parse(&head_buff);
        let mut body = vec![0; header.len as usize];
        reader.read_exact(&mut body).await?;

       Ok(Some((header,body)))
    //}

}

async fn process(
    op_to_chain: mpsc::UnboundedSender<UnverifiedTransaction>,
    mut op_from_chain:  mpsc::UnboundedReceiver<H256>,
    mut op_from_net:  mpsc::UnboundedReceiver<(Header,Vec<u8>)>,
    iot: Arc<Mutex<Iot>>,
) -> io::Result<()> {
    //println!("Accepted from: {}", net_writer.peer_addr()?);

    //let res = iot.lock().await.proc_body(header.ptype, &body);

    // if res {
    //     let tmp = vec![1, 2];
    //     stream.write(&tmp).await?;
    // } else {
    //     //break;
    // }
    select! {
        tx_hash = op_from_chain.next().fuse() => match tx_hash {
            _ => {}
        },
        payload = network_process(iot.lock().await.tcp.clone()).fuse() => {

        }
        // _ =fut => {

        // }
    }
   
    if let Some(mut net_writer) = iot.lock().await.tcp.clone() {
        let tmp = vec![1, 2];
        net_writer.write(&tmp).await?;
    }
    
    
    Ok(())
}

async fn chain_loop(
    chain_to_op: mpsc::UnboundedSender<H256>,
    chain_from_op: Receiver<UnverifiedTransaction>,
) {
}

fn main() -> io::Result<()> {
    //let (chain_sender,from_chain) = mpsc::unbounded::<H256>();
    //let iot_static: Arc<Mutex<Iot>> = Arc::new(Mutex::new(Iot::new()));
    

    task::block_on(async {
        let listener = TcpListener::bind("0.0.0.0:18888").await?;
        println!("Listening on {}", listener.local_addr()?);

        let (op_to_chain,mut  chain_from_op) = mpsc::unbounded();
        let (chain_to_op, mut op_from_chain) = mpsc::unbounded();
        let (net_to_op, mut op_from_net) = mpsc::unbounded();

        let _task = task::spawn(chain_loop(chain_to_op.clone(), chain_from_op));

        let ptsk = task::spawn(async {
            if let Ok(_) = process(op_to_chain, op_from_chain, op_from_net, ONE_IOT.clone()).await {
                //;
            }
        });

        let mut incoming = listener.incoming();
        while let Some(stream) = incoming.next().await {
            let stream = stream?;
            let iot = ONE_IOT.clone();
            if iot.lock().await.used == false {
                iot.lock().await.used = true;
                iot.lock().await.set_stream(stream.clone());
            }
        }
        ptsk.await;
        Ok(())
    })
}
