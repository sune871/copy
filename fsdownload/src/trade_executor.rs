use anyhow::{Result, Context};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    instruction::Instruction,
    message::Message,
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
    system_instruction,
    transaction::Transaction,
};
use solana_transaction_status::UiTransactionEncoding;
use std::str::FromStr;
use tracing::{info, warn, error};
use crate::types::{TradeDetails, TradeDirection, TradeExecutionConfig, ExecutedTrade, DexType};
use chrono::Utc;

#[derive(Clone)]
pub struct TradeExecutor {
    client: RpcClient,
    copy_wallet: Keypair,
    config: TradeExecutionConfig,
}

impl TradeExecutor {
    pub fn new(rpc_url: &str, config: TradeExecutionConfig) -> Result<Self> {
        let client = RpcClient::new_with_commitment(
            rpc_url.to_string(),
            CommitmentConfig::confirmed(),
        );
        
        // 从私钥创建钱包
        let private_key_bytes = bs58::decode(&config.copy_wallet_private_key)
            .into_vec()
            .context("无法解码私钥")?;
        
        let copy_wallet = Keypair::from_bytes(&private_key_bytes)
            .context("无法从私钥创建钱包")?;
        
        info!("交易执行器初始化完成，钱包地址: {}", copy_wallet.pubkey());
        
        Ok(TradeExecutor {
            client,
            copy_wallet,
            config,
        })
    }
    
    /// 执行跟单交易
    pub async fn execute_trade(&self, trade: &TradeDetails) -> Result<ExecutedTrade> {
        if !self.config.enabled {
            return Ok(ExecutedTrade {
                original_signature: trade.signature.clone(),
                copy_signature: "".to_string(),
                trade_direction: trade.trade_direction.clone(),
                amount_in: trade.amount_in,
                amount_out: trade.amount_out,
                price: trade.price,
                gas_fee: trade.gas_fee,
                timestamp: Utc::now().timestamp(),
                success: false,
                error_message: Some("交易执行已禁用".to_string()),
            });
        }
        
        // 检查交易金额是否在允许范围内
        let trade_amount_sol = trade.amount_in as f64 / 1_000_000_000.0;
        if trade_amount_sol < self.config.min_trade_amount {
            return Ok(ExecutedTrade {
                original_signature: trade.signature.clone(),
                copy_signature: "".to_string(),
                trade_direction: trade.trade_direction.clone(),
                amount_in: trade.amount_in,
                amount_out: trade.amount_out,
                price: trade.price,
                gas_fee: trade.gas_fee,
                timestamp: Utc::now().timestamp(),
                success: false,
                error_message: Some(format!("交易金额 {} SOL 小于最小金额 {} SOL", 
                    trade_amount_sol, self.config.min_trade_amount)),
            });
        }
        
        if trade_amount_sol > self.config.max_trade_amount {
            return Ok(ExecutedTrade {
                original_signature: trade.signature.clone(),
                copy_signature: "".to_string(),
                trade_direction: trade.trade_direction.clone(),
                amount_in: trade.amount_in,
                amount_out: trade.amount_out,
                price: trade.price,
                gas_fee: trade.gas_fee,
                timestamp: Utc::now().timestamp(),
                success: false,
                error_message: Some(format!("交易金额 {} SOL 大于最大金额 {} SOL", 
                    trade_amount_sol, self.config.max_trade_amount)),
            });
        }
        
        info!("开始执行跟单交易:");
        info!("  原始交易: {}", trade.signature);
        info!("  交易方向: {:?}", trade.trade_direction);
        info!("  交易金额: {:.6} SOL", trade_amount_sol);
        info!("  代币: {:?}", trade.token_out.symbol);
        
        match trade.dex_type {
            DexType::RaydiumCPMM => {
                self.execute_raydium_cpmm_trade(trade).await
            }
            DexType::PumpFun => {
                self.execute_pump_trade(trade).await
            }
            _ => {
                warn!("不支持的DEX类型: {:?}", trade.dex_type);
                Ok(ExecutedTrade {
                    original_signature: trade.signature.clone(),
                    copy_signature: "".to_string(),
                    trade_direction: trade.trade_direction.clone(),
                    amount_in: trade.amount_in,
                    amount_out: trade.amount_out,
                    price: trade.price,
                    gas_fee: trade.gas_fee,
                    timestamp: Utc::now().timestamp(),
                    success: false,
                    error_message: Some(format!("不支持的DEX类型: {:?}", trade.dex_type)),
                })
            }
        }
    }
    
    /// 执行Raydium CPMM交易
    async fn execute_raydium_cpmm_trade(&self, trade: &TradeDetails) -> Result<ExecutedTrade> {
        // 这里需要实现具体的Raydium CPMM交易逻辑
        // 由于涉及复杂的DEX交互，这里提供一个基础框架
        
        info!("执行Raydium CPMM交易...");
        
        // 获取最新区块哈希
        let recent_blockhash = self.client.get_latest_blockhash()?;
        
        // 创建交易指令（这里需要根据具体的DEX协议实现）
        let instructions = self.create_raydium_cpmm_instructions(trade)?;
        
        // 创建交易
        let message = Message::new(&instructions, Some(&self.copy_wallet.pubkey()));
        let mut transaction = Transaction::new_unsigned(message);
        
        // 签名交易
        transaction.sign(&[&self.copy_wallet], recent_blockhash);
        
        // 发送交易
        match self.client.send_and_confirm_transaction(&transaction) {
            Ok(signature) => {
                info!("跟单交易成功: {}", signature);
                Ok(ExecutedTrade {
                    original_signature: trade.signature.clone(),
                    copy_signature: signature.to_string(),
                    trade_direction: trade.trade_direction.clone(),
                    amount_in: trade.amount_in,
                    amount_out: trade.amount_out,
                    price: trade.price,
                    gas_fee: trade.gas_fee,
                    timestamp: Utc::now().timestamp(),
                    success: true,
                    error_message: None,
                })
            }
            Err(e) => {
                error!("跟单交易失败: {}", e);
                Ok(ExecutedTrade {
                    original_signature: trade.signature.clone(),
                    copy_signature: "".to_string(),
                    trade_direction: trade.trade_direction.clone(),
                    amount_in: trade.amount_in,
                    amount_out: trade.amount_out,
                    price: trade.price,
                    gas_fee: trade.gas_fee,
                    timestamp: Utc::now().timestamp(),
                    success: false,
                    error_message: Some(e.to_string()),
                })
            }
        }
    }
    
    /// 执行Pump.fun交易
    async fn execute_pump_trade(&self, trade: &TradeDetails) -> Result<ExecutedTrade> {
        info!("执行Pump.fun交易...");
        
        // 获取最新区块哈希
        let recent_blockhash = self.client.get_latest_blockhash()?;
        
        // 创建交易指令
        let instructions = self.create_pump_instructions(trade)?;
        
        // 创建交易
        let message = Message::new(&instructions, Some(&self.copy_wallet.pubkey()));
        let mut transaction = Transaction::new_unsigned(message);
        
        // 签名交易
        transaction.sign(&[&self.copy_wallet], recent_blockhash);
        
        // 发送交易
        match self.client.send_and_confirm_transaction(&transaction) {
            Ok(signature) => {
                info!("跟单交易成功: {}", signature);
                Ok(ExecutedTrade {
                    original_signature: trade.signature.clone(),
                    copy_signature: signature.to_string(),
                    trade_direction: trade.trade_direction.clone(),
                    amount_in: trade.amount_in,
                    amount_out: trade.amount_out,
                    price: trade.price,
                    gas_fee: trade.gas_fee,
                    timestamp: Utc::now().timestamp(),
                    success: true,
                    error_message: None,
                })
            }
            Err(e) => {
                error!("跟单交易失败: {}", e);
                Ok(ExecutedTrade {
                    original_signature: trade.signature.clone(),
                    copy_signature: "".to_string(),
                    trade_direction: trade.trade_direction.clone(),
                    amount_in: trade.amount_in,
                    amount_out: trade.amount_out,
                    price: trade.price,
                    gas_fee: trade.gas_fee,
                    timestamp: Utc::now().timestamp(),
                    success: false,
                    error_message: Some(e.to_string()),
                })
            }
        }
    }
    
    /// 创建Raydium CPMM交易指令
    fn create_raydium_cpmm_instructions(&self, trade: &TradeDetails) -> Result<Vec<Instruction>> {
        // 这里需要实现具体的Raydium CPMM指令创建逻辑
        // 由于涉及复杂的DEX协议，这里提供一个基础框架
        
        let mut instructions = Vec::new();
        
        match trade.trade_direction {
            TradeDirection::Buy => {
                // 买入代币的指令
                info!("创建买入指令");
                // TODO: 实现具体的买入指令
            }
            TradeDirection::Sell => {
                // 卖出代币的指令
                info!("创建卖出指令");
                // TODO: 实现具体的卖出指令
            }
        }
        
        Ok(instructions)
    }
    
    /// 创建Pump.fun交易指令
    fn create_pump_instructions(&self, trade: &TradeDetails) -> Result<Vec<Instruction>> {
        let mut instructions = Vec::new();
        
        match trade.trade_direction {
            TradeDirection::Buy => {
                // 买入Pump.fun代币
                info!("创建Pump.fun买入指令");
                // TODO: 实现具体的Pump.fun买入指令
            }
            TradeDirection::Sell => {
                // 卖出Pump.fun代币
                info!("创建Pump.fun卖出指令");
                // TODO: 实现具体的Pump.fun卖出指令
            }
        }
        
        Ok(instructions)
    }
    
    /// 获取钱包余额
    pub fn get_wallet_balance(&self) -> Result<f64> {
        let balance = self.client.get_balance(&self.copy_wallet.pubkey())?;
        Ok(balance as f64 / 1_000_000_000.0)
    }
    
    /// 检查钱包是否有足够余额
    pub fn check_balance(&self, required_amount: u64) -> Result<bool> {
        let balance = self.client.get_balance(&self.copy_wallet.pubkey())?;
        Ok(balance >= required_amount)
    }
} 