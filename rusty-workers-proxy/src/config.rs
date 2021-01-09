use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use rusty_workers::app::AppConfig;

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct Config {
    pub apps: Vec<AppConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            apps: Default::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LocalConfig {
    pub runtime_cluster: Vec<SocketAddr>,
    pub max_ready_instances_per_app: usize,
    pub ready_instance_expiration_ms: u64,
    pub request_timeout_ms: u64,
    pub max_request_body_size_bytes: u64,
    pub dropout_rate: f32,
}
