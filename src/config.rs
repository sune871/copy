use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub rpc_url: String,
    pub target_wallets: Vec<String>,
    pub copy_wallet_private_key: String,
    pub trading_settings: TradingSettings,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TradingSettings {
    pub max_position_size: f64,
    pub slippage_tolerance: f64,
    pub gas_price_multiplier: f64,
}

impl Config {
    pub fn load() -> Result<Self> {
        let config_str = fs::read_to_string("config.json")?;
        let config: Config = serde_json::from_str(&config_str)?;
        Ok(config)
    }
}