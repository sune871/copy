// https://solana-rpc.publicnode.com/f884f7c2cfa0e7ecbf30e7da70ec1da91bda3c9d04058269397a5591e7fd013e";
// CuwxHwz42cNivJqWGBk6HcVvfGq47868Mo6zi4u6z9vC

mod parser;
mod types;
mod grpc_monitor;
mod dex;
mod config;
mod trade_executor;
mod trade_recorder;

use anyhow::Result;
use grpc_monitor::GrpcMonitor;
use trade_executor::TradeExecutor;
use trade_recorder::TradeRecorder;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use tracing::{info, error};

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志系统
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    
    info!("启动Solana钱包监控和跟单程序");
    
    // 加载配置
    let config = config::Config::load()?;
    info!("配置加载成功");
    
    // 创建交易记录器
    let recorder = TradeRecorder::new("trades/trade_records.json");
    recorder.ensure_directory()?;
    info!("交易记录器初始化完成");
    
    // 创建交易执行器
    let executor = TradeExecutor::new(&config.rpc_url, config.get_execution_config())?;
    
    // 显示钱包余额
    let balance = executor.get_wallet_balance()?;
    info!("跟单钱包余额: {:.6} SOL", balance);
    
    // 配置信息
    let grpc_endpoint = "https://solana-yellowstone-grpc.publicnode.com:443";
    let auth_token = Some("your-auth-token".to_string());
    let wallet_address = &config.target_wallets[0];
    let wallet_pubkey = Pubkey::from_str(wallet_address)?;
    
    // 创建gRPC监控器（传入交易执行器和记录器）
    let monitor = GrpcMonitor::new_with_executor(
        grpc_endpoint.to_string(), 
        auth_token, 
        wallet_pubkey,
        executor,
    );
    
    // 启动监控
    match monitor.start_monitoring().await {
        Ok(_) => info!("监控程序正常结束"),
        Err(e) => error!("监控程序出错: {}", e),
    }
    
    Ok(())
}