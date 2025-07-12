use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeDetails {
    pub signature: String,
    pub wallet: Pubkey,
    pub dex_program: String,
    pub input_token: Pubkey,
    pub output_token: Pubkey,
    pub amount_in: u64,
    pub amount_out: u64,
    pub price: f64,
    pub timestamp: i64,
}

#[derive(Debug, Clone)]
pub enum DexType {
    Raydium,
    PumpFun,
    Unknown,
}