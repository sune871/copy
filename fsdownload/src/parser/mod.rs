pub mod raydium;
pub mod pump;
pub mod raydium_cpmm;

use anyhow::Result;
use tracing::{info, warn};
use crate::types::{TradeDetails, DexType, RAYDIUM_AMM_V4, PUMP_FUN_PROGRAM, RAYDIUM_CPMM, RAYDIUM_CLMM};

pub struct TransactionParser;

impl TransactionParser {
    pub fn new() -> Self {
        TransactionParser
    }
    
    /// 从交易数据中解析交易详情
    pub fn parse_transaction_data(
        &self,
        signature: &str,
        account_keys: &[String],
        instruction_data: &[u8],
        pre_balances: &[u64],
        post_balances: &[u64],
        pre_token_balances: &[serde_json::Value],
        post_token_balances: &[serde_json::Value],
        logs: &[String],
    ) -> Result<Option<TradeDetails>> {
        // 识别DEX类型
        let dex_type = self.identify_dex_from_accounts(account_keys)?;
        
        match dex_type {
            DexType::RaydiumAmmV4 => {
                info!("检测到Raydium交易，开始解析...");
                raydium::parse_raydium_amm_v4_swap(
                    signature,
                    account_keys,
                    instruction_data,
                    pre_balances,
                    post_balances,
                    pre_token_balances,
                    post_token_balances,
                    logs,
                )
            }
            DexType::PumpFun => {
                info!("检测到Pump.fun交易，开始解析...");
                pump::parse_pump_trade(
                    signature,
                    account_keys,
                    instruction_data,
                    pre_balances,
                    post_balances,
                    pre_token_balances,
                    post_token_balances,
                    logs,
                )
            }
            DexType::RaydiumCPMM => {
                info!("检测到Raydium CPMM交易，开始解析...");
                raydium_cpmm::parse_raydium_cpmm_swap(
                    signature,
                    account_keys,
                    instruction_data,
                    pre_balances,
                    post_balances,
                    pre_token_balances,
                    post_token_balances,
                    logs,
                )
            }
            DexType::RaydiumCLMM => {
                info!("检测到Raydium CLMM交易，开始解析...");
                raydium::parse_raydium_amm_v4_swap(
                    signature,
                    account_keys,
                    instruction_data,
                    pre_balances,
                    post_balances,
                    pre_token_balances,
                    post_token_balances,
                    logs,
                )
            }
            
            DexType::Unknown => {
                warn!("未识别的DEX类型，跳过解析");
                Ok(None)
            }
        }
    }
    
    /// 从账户列表中识别DEX类型
    fn identify_dex_from_accounts(&self, account_keys: &[String]) -> Result<DexType> {
        // 新增：支持指令级别的program_id判断
        // 这里假设你能传入当前指令的program_id_index（如需更精细可扩展参数）
        // 先用原有逻辑
        for account in account_keys {
            if account == RAYDIUM_AMM_V4 {
                return Ok(DexType::RaydiumAmmV4);
            } else if account == PUMP_FUN_PROGRAM {
                return Ok(DexType::PumpFun);
            } else if account == RAYDIUM_CPMM {
                return Ok(DexType::RaydiumCPMM);
            } else if account == RAYDIUM_CLMM {
                return Ok(DexType::RaydiumCLMM);
            }
        }
        Ok(DexType::Unknown)
    }
    
    /// 辅助函数：查找账户索引
    pub fn find_account_index(account_keys: &[String], target: &str) -> Option<usize> {
        account_keys.iter().position(|key| key == target)
    }
    
    /// 辅助函数：计算代币余额变化
    pub fn calculate_token_balance_change(
        pre_balances: &[serde_json::Value],
        post_balances: &[serde_json::Value],
        mint: &str,
    ) -> Result<(u64, u64)> {
        let mut pre_amount = 0u64;
        let mut post_amount = 0u64;
        
        // 查找指定mint的余额
        for balance in pre_balances {
            if let Some(balance_mint) = balance.get("mint").and_then(|m| m.as_str()) {
                if balance_mint == mint {
                    pre_amount = balance
                        .get("uiTokenAmount")
                        .and_then(|ui| ui.get("amount"))
                        .and_then(|a| a.as_str())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(0);
                    break;
                }
            }
        }
        
        for balance in post_balances {
            if let Some(balance_mint) = balance.get("mint").and_then(|m| m.as_str()) {
                if balance_mint == mint {
                    post_amount = balance
                        .get("uiTokenAmount")
                        .and_then(|ui| ui.get("amount"))
                        .and_then(|a| a.as_str())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(0);
                    break;
                }
            }
        }
        
        Ok((pre_amount, post_amount))
    }
    
    /// 辅助函数：从日志中提取交易信息
    pub fn extract_info_from_logs(logs: &[String], pattern: &str) -> Option<String> {
        for log in logs {
            if log.contains(pattern) {
                return Some(log.clone());
            }
        }
        None
    }
}