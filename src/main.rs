// https://solana-rpc.publicnode.com/f884f7c2cfa0e7ecbf30e7da70ec1da91bda3c9d04058269397a5591e7fd013e";
// CuwxHwz42cNivJqWGBk6HcVvfGq47868Mo6zi4u6z9vC

mod parser;
mod types;
mod grpc_monitor;

use anyhow::Result;
use grpc_monitor::GrpcMonitor;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use tracing::{info, error};

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志系统
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    
    info!("启动Solana钱包监控程序 (gRPC模式)");
    
    // 配置信息
    let grpc_endpoint = "https://solana-yellowstone-grpc.publicnode.com:443"; // 需要替换为实际的gRPC端点
    let auth_token = Some("your-auth-token".to_string()); // 如果需要认证令牌
    let wallet_address = "CuwxHwz42cNivJqWGBk6HcVvfGq47868Mo6zi4u6z9vC";
    let wallet_pubkey = Pubkey::from_str(wallet_address)?;
    
    // 创建gRPC监控器
    let monitor = GrpcMonitor::new(grpc_endpoint.to_string(), auth_token, wallet_pubkey);
    
    // 启动监控
    match monitor.start_monitoring().await {
        Ok(_) => info!("gRPC监控正常结束"),
        Err(e) => error!("gRPC监控出错: {}", e),
    }
    
    Ok(())
}