use anyhow::Result;
use solana_client::rpc_client::RpcClient;
use solana_sdk::signature::Signature;
use std::str::FromStr;
use tracing::{info, warn};

// Raydium AMM程序地址
const RAYDIUM_AMM_PROGRAM: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";
// Pump.fun程序地址
const PUMP_PROGRAM: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwdFi";

pub struct DexDetector {
    client: RpcClient,
}

impl DexDetector {
    pub fn new(rpc_url: &str) -> Self {
        DexDetector {
            client: RpcClient::new(rpc_url.to_string()),
        }
    }
    
    // 检测交易是否涉及特定DEX
    pub fn check_transaction_dex(&self, signature_str: &str) -> Result<()> {
        let signature = Signature::from_str(signature_str)?;
        
        info!("检查交易中的DEX活动: {}", signature_str);
        
        // 获取交易
        match self.client.get_transaction(
            &signature, 
            solana_transaction_status::UiTransactionEncoding::Base64
        ) {
            Ok(confirmed_tx) => {
                // 检查交易是否成功
                if let Some(meta) = &confirmed_tx.transaction.meta {
                    if meta.err.is_some() {
                        warn!("交易失败，跳过分析");
                        return Ok(());
                    }
                    
                    // 显示基本信息
                    let fee_sol = meta.fee as f64 / 1_000_000_000.0;
                    info!("交易费用: {} SOL", fee_sol);
                    
                    // 这里我们暂时只记录交易存在
                    // 在实际应用中，您需要解析交易数据来识别具体的DEX操作
                    info!("交易分析完成");
                }
            }
            Err(e) => {
                warn!("无法获取交易详情: {}", e);
            }
        }
        
        Ok(())
    }
}