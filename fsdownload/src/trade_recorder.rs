use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use tracing::info;
use crate::types::{TradeDetails, ExecutedTrade};
use chrono::{DateTime, Utc};

#[derive(Debug, Serialize, Deserialize)]
pub struct TradeRecord {
    pub timestamp: DateTime<Utc>,
    pub original_signature: String,
    pub copy_signature: Option<String>,
    pub trade_direction: String,
    pub dex_type: String,
    pub token_in_symbol: Option<String>,
    pub token_out_symbol: Option<String>,
    pub amount_in: u64,
    pub amount_out: u64,
    pub price: f64,
    pub gas_fee: u64,
    pub success: bool,
    pub error_message: Option<String>,
}

pub struct TradeRecorder {
    file_path: String,
}

impl TradeRecorder {
    pub fn new(file_path: &str) -> Self {
        TradeRecorder {
            file_path: file_path.to_string(),
        }
    }
    
    /// 记录原始交易
    pub fn record_trade(&self, trade: &TradeDetails) -> Result<()> {
        let record = TradeRecord {
            timestamp: Utc::now(),
            original_signature: trade.signature.clone(),
            copy_signature: None,
            trade_direction: format!("{:?}", trade.trade_direction),
            dex_type: format!("{:?}", trade.dex_type),
            token_in_symbol: trade.token_in.symbol.clone(),
            token_out_symbol: trade.token_out.symbol.clone(),
            amount_in: trade.amount_in,
            amount_out: trade.amount_out,
            price: trade.price,
            gas_fee: trade.gas_fee,
            success: true,
            error_message: None,
        };
        
        self.write_record(&record)
    }
    
    /// 记录执行结果
    pub fn record_execution(&self, executed_trade: &ExecutedTrade) -> Result<()> {
        let record = TradeRecord {
            timestamp: Utc::now(),
            original_signature: executed_trade.original_signature.clone(),
            copy_signature: if executed_trade.copy_signature.is_empty() {
                None
            } else {
                Some(executed_trade.copy_signature.clone())
            },
            trade_direction: format!("{:?}", executed_trade.trade_direction),
            dex_type: "Unknown".to_string(),
            token_in_symbol: None,
            token_out_symbol: None,
            amount_in: executed_trade.amount_in,
            amount_out: executed_trade.amount_out,
            price: executed_trade.price,
            gas_fee: executed_trade.gas_fee,
            success: executed_trade.success,
            error_message: executed_trade.error_message.clone(),
        };
        
        self.write_record(&record)
    }
    
    fn write_record(&self, record: &TradeRecord) -> Result<()> {
        let json = serde_json::to_string_pretty(record)?;
        
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file_path)?;
        
        writeln!(file, "{}", json)?;
        file.flush()?;
        
        info!("交易记录已保存到: {}", self.file_path);
        Ok(())
    }
    
    /// 创建trades目录
    pub fn ensure_directory(&self) -> Result<()> {
        if let Some(parent) = Path::new(&self.file_path).parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }
} 