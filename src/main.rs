
mod payload;

use async_std::io::{self,BufReader};
use async_std::net::{TcpListener, TcpStream};
use async_std::prelude::*;
use async_std::task;
use std::sync::{Arc,
    atomic::{AtomicBool, Ordering}
};
use payload::Header;
//#[macro_use]
//use lazy_static;

lazy_static::lazy_static!{
    static ref ONE_CONN_FLAG:Arc<AtomicBool> = Arc::new(AtomicBool::new(true));
}

fn proc_body(ptype: u8,body :&[u8]) {
    match ptype {
        
    }
}


async fn process(stream: TcpStream,is_once : Arc<AtomicBool>) -> io::Result<()> {
    println!("Accepted from: {}", stream.peer_addr()?);

    if !is_once.fetch_and(false, Ordering::SeqCst) {
        return Err(io::Error::from(io::ErrorKind::AlreadyExists));
    }
    let mut reader = BufReader::new(stream.clone());
    let mut head_buff = vec![0;8];
    reader.read_exact(&mut head_buff).await?;
   
    let header = Header::parse(&head_buff);
    let mut body = vec![0;header.len as usize];
    reader.read_exact(&mut body).await?;


    
    

    Ok(())
}

fn main() -> io::Result<()> {

    task::block_on(async {
        let listener = TcpListener::bind("0.0.0.0:18888").await?;
        println!("Listening on {}", listener.local_addr()?);
        
        let mut incoming = listener.incoming();

        while let Some(stream) = incoming.next().await {
            let stream = stream?;
            
            task::spawn(async {
                process(stream,ONE_CONN_FLAG.clone()).await.unwrap();
            });
        }
        Ok(())
    })
}
