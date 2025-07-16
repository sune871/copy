// https://solana-rpc.publicnode.com/f884f7c2cfa0e7ecbf30e7da70ec1da91bda3c9d04058269397a5591e7fd013e";
// CuwxHwz42cNivJqWGBk6HcVvfGq47868Mo6zi4u6z9vC

mod parser;
mod types;
mod grpc_monitor;
mod dex;
mod config;
mod trade_executor;
mod trade_recorder;
mod test_runner;
mod mock_monitor;

use anyhow::Result;
use grpc_monitor::GrpcMonitor;
use trade_executor::TradeExecutor;
use trade_recorder::TradeRecorder;
use test_runner::TestRunner;
use mock_monitor::MockMonitor;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use tracing::{info, error, warn};
use spl_associated_token_account::get_associated_token_address;
use solana_sdk::signature::Keypair;
use solana_client::rpc_client::RpcClient;
use anyhow::Context;
use solana_sdk::signer::Signer;
use std::process::Command;

fn check_wsol_balance_or_exit(rpc: &RpcClient, wallet: &Keypair, min_required: u64) {
    let wsol_mint = Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap();
    let wsol_ata = get_associated_token_address(&wallet.pubkey(), &wsol_mint);
    let wsol_balance = rpc.get_token_account_balance(&wsol_ata)
        .map(|b| b.amount.parse::<u64>().unwrap_or(0))
        .unwrap_or(0);
    if wsol_balance < min_required {
        tracing::error!("[启动检查] 跟单钱包WSOL余额不足，当前余额: {}，请手动补充WSOL后再启动！", wsol_balance);
        std::process::exit(1);
    } else {
        tracing::info!("[启动检查] 跟单钱包WSOL余额充足: {}", wsol_balance);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志系统
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    
    info!("🚀 启动Solana钱包监控和跟单程序");
    
    // 检查命令行参数
    let args: Vec<String> = std::env::args().collect();
    
    if args.len() > 1 {
        match args[1].as_str() {
            "--test" | "-t" => {
                info!("🧪 运行测试模式...");
                return run_test_mode().await;
            }
            "--performance" | "-p" => {
                info!("⚡ 运行性能测试...");
                return run_performance_test().await;
            }
            "--mock" | "-m" => {
                info!("🎭 运行模拟监控模式...");
                return run_mock_mode().await;
            }
            "--update-pools" => {
                info!("⏬ 正在拉取最新池子参数...");
                let status = Command::new("cargo")
                    .args(&["run", "--bin", "fetch_pools"])
                    .status()
                    .expect("failed to update pools");
                if status.success() {
                    println!("池子参数已成功更新！");
                } else {
                    eprintln!("池子参数更新失败，请检查fetch_pools脚本和网络连接。");
                }
                return Ok(());
            }
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            _ => {
                error!("未知参数: {}", args[1]);
                print_usage();
                return Ok(());
            }
        }
    }
    
    // 读取配置，初始化钱包和RPC
    let config = config::Config::load()?;
    let rpc_client = RpcClient::new_with_commitment(
        config.rpc_url.clone(),
        solana_sdk::commitment_config::CommitmentConfig::confirmed(),
    );
    let private_key_bytes = bs58::decode(&config.copy_wallet_private_key)
        .into_vec()
        .context("无法解码私钥")?;
    let copy_wallet = Keypair::from_bytes(&private_key_bytes)
        .context("无法从私钥创建钱包")?;
    // ====== 启动时检测WSOL余额 ======
    let min_required = 10_000_000; // 0.01 SOL，或自定义
    check_wsol_balance_or_exit(&rpc_client, &copy_wallet, min_required);
    
    // 正常运行模式
    run_normal_mode().await
}

/// 运行测试模式
async fn run_test_mode() -> Result<()> {
    let test_runner = TestRunner::new()?;
    test_runner.run_all_tests().await
}

/// 运行性能测试
async fn run_performance_test() -> Result<()> {
    let test_runner = TestRunner::new()?;
    test_runner.run_performance_test()
}

/// 运行模拟监控模式
async fn run_mock_mode() -> Result<()> {
    // 加载配置
    let config = config::Config::load()?;
    info!("配置加载成功");
    
    // 获取目标钱包
    let wallet_address = &config.target_wallets[0];
    let wallet_pubkey = Pubkey::from_str(wallet_address)?;
    
    // 创建模拟监控器
    let mut mock_monitor = MockMonitor::new(wallet_pubkey)?;
    
    // 启动模拟监控
    match mock_monitor.start_monitoring().await {
        Ok(_) => info!("模拟监控正常结束"),
        Err(e) => error!("模拟监控出错: {}", e),
    }
    
    Ok(())
}

/// 正常运行模式
async fn run_normal_mode() -> Result<()> {
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
    match executor.get_wallet_balance() {
        Ok(balance) => {
            info!("跟单钱包余额: {:.6} SOL", balance);
        }
        Err(e) => {
            warn!("无法获取钱包余额: {}", e);
        }
    }
    
    // 配置信息
    let grpc_endpoint = "https://solana-yellowstone-grpc.publicnode.com:443";
    let auth_token = Some("your-auth-token".to_string());
    let wallet_address = &config.target_wallets[0];
    let wallet_pubkey = Pubkey::from_str(wallet_address)?;
    
    // 创建gRPC监控器（传入交易执行器和记录器）
    let monitor = GrpcMonitor::new_with_executor_and_recorder(
        grpc_endpoint.to_string(),
        auth_token,
        wallet_pubkey,
        std::sync::Arc::new(executor),
        recorder,
    );
    
    // 启动监控
    match monitor.start_monitoring().await {
        Ok(_) => info!("监控程序正常结束"),
        Err(e) => error!("监控程序出错: {}", e),
    }
    
    Ok(())
}

/// 打印使用说明
fn print_usage() {
    println!("Solana钱包监控和跟单程序");
    println!();
    println!("使用方法:");
    println!("  cargo run                    # 正常运行模式");
    println!("  cargo run --test             # 运行测试模式");
    println!("  cargo run --performance      # 运行性能测试");
    println!("  cargo run --mock             # 运行模拟监控模式");
    println!("  cargo run --update-pools     # 拉取最新池子参数");
    println!("  cargo run --help             # 显示此帮助信息");
    println!();
    println!("模式说明:");
    println!("  正常运行模式: 连接真实gRPC服务，监控真实交易");
    println!("  测试模式: 验证程序核心功能，无需网络连接");
    println!("  性能测试: 模拟处理1000个交易并测量性能");
    println!("  模拟监控: 生成模拟交易数据，测试交易处理流程");
    println!();
    println!("测试模式将验证:");
    println!("  - 配置加载和验证");
    println!("  - 交易解析功能");
    println!("  - 交易记录功能");
    println!("  - 模拟交易处理");
}