use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Debug)]
pub struct AllConfig {
    pub conf: Config,
    pub dev_conf: DevAddr,
    pub dev_file: String,
    pub client_start: bool,
}

#[derive(Clone, Deserialize, Default, Debug)]
pub struct Config {
    pub url: String,
    pub account: String,
}

#[derive(Clone, Serialize, Deserialize, Default, Debug)]
pub struct DevAddr {
    pub dev_addr: String,
    pub dev_pk: String,
}
