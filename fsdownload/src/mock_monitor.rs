use anyhow::Result;
use tracing::{info, warn};
use crate::types::{TradeDetails, TradeDirection, DexType, TokenInfo};
use crate::trade_recorder::TradeRecorder;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;

pub struct MockMonitor {
    target_wallet: Pubkey,
    recorder: TradeRecorder,
    running: bool,
}

impl MockMonitor {
    pub fn new(target_wallet: Pubkey) -> Result<Self> {
        let recorder = TradeRecorder::new("mock_trades.json");
        recorder.ensure_directory()?;
        
        Ok(MockMonitor {
            target_wallet,
            recorder,
            running: false,
        })
    }
    
    /// 启动模拟监控
    pub async fn start_monitoring(&mut self) -> Result<()> {
        info!("🎭 启动模拟监控模式");
        info!("目标钱包: {}", self.target_wallet);
        
        self.running = true;
        
        // 模拟监控循环
        let mut counter = 0;
        while self.running && counter < 10 {
            counter += 1;
            info!("📡 模拟监控循环 #{}", counter);
            
            // 生成模拟交易
            let mock_trade = self.generate_mock_trade(counter)?;
            
            // 处理交易
            self.handle_mock_trade(mock_trade).await?;
            
            // 等待一段时间
            sleep(Duration::from_secs(2)).await;
        }
        
        info!("✅ 模拟监控完成");
        Ok(())
    }
    
    /// 停止监控
    pub fn stop_monitoring(&mut self) {
        self.running = false;
        info!("🛑 停止模拟监控");
    }
    
    /// 生成模拟交易
    fn generate_mock_trade(&self, counter: u32) -> Result<TradeDetails> {
        let trade_types = vec![
            (TradeDirection::Buy, DexType::RaydiumCPMM, "SOL", "USDC", 1.0, 25.0),
            (TradeDirection::Sell, DexType::RaydiumCPMM, "USDC", "SOL", 25.0, 1.0),
            (TradeDirection::Buy, DexType::PumpFun, "SOL", "PUMP", 0.5, 1000.0),
            (TradeDirection::Sell, DexType::PumpFun, "PUMP", "SOL", 1000.0, 0.5),
        ];
        
        let (direction, dex_type, token_in_symbol, token_out_symbol, amount_in_sol, amount_out_token) = 
            trade_types[counter as usize % trade_types.len()].clone();
        
        let trade = TradeDetails {
            signature: format!("mock_trade_{}", counter),
            wallet: self.target_wallet,
            dex_type,
            trade_direction: direction,
            token_in: TokenInfo {
                mint: Pubkey::from_str("So11111111111111111111111111111111111111112")?,
                symbol: Some(token_in_symbol.to_string()),
                decimals: 9,
            },
            token_out: TokenInfo {
                mint: Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")?,
                symbol: Some(token_out_symbol.to_string()),
                decimals: 6,
            },
            amount_in: (amount_in_sol * 1_000_000_000.0) as u64,
            amount_out: (amount_out_token * 1_000_000.0) as u64,
            price: amount_in_sol / amount_out_token,
            pool_address: Pubkey::from_str("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8")?,
            timestamp: chrono::Utc::now().timestamp(),
            gas_fee: 5_000,
            program_id: Pubkey::from_str("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8")?,
        };
        
        Ok(trade)
    }
    
    /// 处理模拟交易
    async fn handle_mock_trade(&self, trade: TradeDetails) -> Result<()> {
        info!("🎯 处理模拟交易: {}", trade.signature);
        info!("  方向: {:?}", trade.trade_direction);
        info!("  DEX: {:?}", trade.dex_type);
        info!("  输入: {} {}", 
            trade.amount_in as f64 / 1_000_000_000.0,
            trade.token_in.symbol.as_ref().unwrap_or(&"未知".to_string())
        );
        info!("  输出: {} {}", 
            trade.amount_out as f64 / 1_000_000.0,
            trade.token_out.symbol.as_ref().unwrap_or(&"未知".to_string())
        );
        info!("  价格: {:.8}", trade.price);
        
        // 记录交易
        match self.recorder.record_trade(&trade) {
            Ok(_) => info!("✅ 交易记录成功"),
            Err(e) => warn!("⚠️ 交易记录失败: {}", e),
        }
        
        // 模拟交易执行延迟
        sleep(Duration::from_millis(100)).await;
        
        info!("✅ 模拟交易处理完成");
        Ok(())
    }
} 