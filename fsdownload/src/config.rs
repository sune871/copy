use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use crate::types::TradeExecutionConfig;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub rpc_url: String,
    pub target_wallets: Vec<String>,
    pub copy_wallet_private_key: String,
    pub trading_settings: TradingSettings,
    pub execution_config: ExecutionConfig,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TradingSettings {
    pub max_position_size: f64,
    pub slippage_tolerance: f64,
    pub gas_price_multiplier: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExecutionConfig {
    pub enabled: bool,
    pub min_trade_amount: f64,
    pub max_trade_amount: f64,
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
    
    pub fn get_execution_config(&self) -> TradeExecutionConfig {
        TradeExecutionConfig {
            copy_wallet_private_key: self.copy_wallet_private_key.clone(),
            max_position_size: self.execution_config.max_position_size,
            slippage_tolerance: self.execution_config.slippage_tolerance,
            gas_price_multiplier: self.execution_config.gas_price_multiplier,
            min_trade_amount: self.execution_config.min_trade_amount,
            max_trade_amount: self.execution_config.max_trade_amount,
            enabled: self.execution_config.enabled,
        }
    }
}