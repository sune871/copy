use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeDetails {
    pub signature: String,
    pub wallet: Pubkey,
    pub dex_type: DexType,
    pub trade_direction: TradeDirection,
    pub token_in: TokenInfo,
    pub token_out: TokenInfo,
    pub amount_in: u64,
    pub amount_out: u64,
    pub price: f64,
    pub pool_address: Pubkey,
    pub timestamp: i64,
    pub gas_fee: u64,
    pub program_id: Pubkey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenInfo {
    pub mint: Pubkey,
    pub symbol: Option<String>,
    pub decimals: u8,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum DexType {
    RaydiumAmmV4,
    RaydiumCPMM,
    RaydiumCLMM,
    PumpFun,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum TradeDirection {
    Buy,    // 用SOL买入代币
    Sell,   // 卖出代币换SOL
}

// 交易执行相关类型
#[derive(Debug, Clone)]
pub struct TradeExecutionConfig {
    pub copy_wallet_private_key: String,
    pub max_position_size: f64,        // 最大仓位大小（SOL）
    pub slippage_tolerance: f64,       // 滑点容忍度
    pub gas_price_multiplier: f64,     // Gas价格倍数
    pub min_trade_amount: f64,         // 最小交易金额（SOL）
    pub max_trade_amount: f64,         // 最大交易金额（SOL）
    pub enabled: bool,                 // 是否启用跟单
}

#[derive(Debug, Clone)]
pub struct ExecutedTrade {
    pub original_signature: String,
    pub copy_signature: String,
    pub trade_direction: TradeDirection,
    pub amount_in: u64,
    pub amount_out: u64,
    pub price: f64,
    pub gas_fee: u64,
    pub timestamp: i64,
    pub success: bool,
    pub error_message: Option<String>,
}

// Raydium AMM V4相关常量
pub const RAYDIUM_AMM_V4: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";
pub const RAYDIUM_AUTHORITY: &str = "5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1";

// Raydium CPMM相关常量
pub const RAYDIUM_CPMM: &str = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";
pub const RAYDIUM_CP_AUTHORITY: &str = "GpMZbSM2GgvTKHJirzeGfMFoaZ8UR2X7F4v8vHTvxFbL";

// Raydium CLMM相关常量
pub const RAYDIUM_CLMM: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";

// Pump.fun相关常量
pub const PUMP_FUN_PROGRAM: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwdFi";

// 代币常量
pub const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
pub const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

// Raydium AMM V4 指令
pub const RAYDIUM_AMM_SWAP_INSTRUCTION: u8 = 9;

// Raydium CPMM 指令
pub const RAYDIUM_CPMM_SWAP_BASE_INPUT: [u8; 8] = [143, 190, 90, 218, 196, 30, 51, 222]; // swap_base_input的discriminator
pub const RAYDIUM_CPMM_SWAP_BASE_OUTPUT: [u8; 8] = [55, 217, 98, 86, 163, 74, 180, 173]; // swap_base_output的discriminator

// Pump.fun 指令
pub const PUMP_BUY_INSTRUCTION: u8 = 0x66;
pub const PUMP_SELL_INSTRUCTION: u8 = 0x33;