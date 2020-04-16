use ethereum_types::H160;

type Address = H160;

const type_HB :u8 = 0;
const type_PK_REQ :u8 = 1;
const type_PK_RES :u8 = 2;
const type_SIGN_REQ :u8 = 3;
const type_SIGN_RES :u8 = 4;
const type_URL_REQ :u8 = 7;
const type_URL_RES :u8 = 8;
const type_ACCOUNT_REQ :u8 = 9;
const type_ACCOUNT_RES :u8 = 10;
const type_TOBE_SENT_DATA :u8 = 11;

#[derive(clone,debug)]
pub struct {
    rpc_url: String,
    account: Address,

}


pub fn proc_body(ptype: u8,body :&[u8]) {
    match ptype {
        type_HB => {
            
        }
    }
}