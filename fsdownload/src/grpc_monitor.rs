use anyhow::{Result, Context};
use futures::{StreamExt, SinkExt};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::bs58;
use std::collections::HashMap;
use tracing::{info, error, warn, debug};
use yellowstone_grpc_client::GeyserGrpcClient;
use yellowstone_grpc_proto::geyser::{
    CommitmentLevel, SubscribeRequest, SubscribeRequestFilterAccounts,
    SubscribeRequestFilterTransactions, SubscribeUpdate, SubscribeUpdateTransaction,
};
use yellowstone_grpc_proto::prelude::{Transaction, Message, TransactionStatusMeta};

// 添加新的导入
use crate::parser::TransactionParser;
use crate::types::TradeDetails;
use serde_json;

// Common DEX program IDs
const RAYDIUM_V4: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";
const JUPITER_V6: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";
const ORCA_WHIRLPOOL: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";

pub struct GrpcMonitor {
    endpoint: String,
    auth_token: Option<String>,
    target_wallet: Pubkey,
}

impl GrpcMonitor {
    pub fn new(endpoint: String, auth_token: Option<String>, target_wallet: Pubkey) -> Self {
        GrpcMonitor {
            endpoint,
            auth_token,
            target_wallet,
        }
    }

    pub async fn start_monitoring(&self) -> Result<()> {
        info!("启动gRPC监控服务，目标钱包: {}", self.target_wallet);
        info!("连接到gRPC端点: {}", self.endpoint);
        
        loop {
            match self.monitor_loop().await {
                Ok(_) => {
                    warn!("监控循环结束，准备重启...");
                }
                Err(e) => {
                    error!("监控错误: {:?}", e);
                }
            }
            
            info!("5秒后重试...");
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }

    async fn monitor_loop(&self) -> Result<()> {
        let mut client = GeyserGrpcClient::build_from_shared(self.endpoint.clone())?
            .connect()
            .await
            .context("无法连接到gRPC服务")?;
        
        info!("已连接到gRPC服务，准备订阅...");
        
        let mut accounts = HashMap::new();
        accounts.insert(
            "wallet".to_string(),
            SubscribeRequestFilterAccounts {
                account: vec![self.target_wallet.to_string()],
                owner: vec![],
                filters: vec![],
            },
        );

        let mut transactions = HashMap::new();
        transactions.insert(
            "wallet_tx".to_string(),
            SubscribeRequestFilterTransactions {
                vote: Some(false),
                failed: Some(false),
                signature: None,
                account_include: vec![self.target_wallet.to_string()],
                account_exclude: vec![],
                account_required: vec![],
            },
        );

        let request = SubscribeRequest {
            accounts,
            slots: HashMap::new(),
            transactions,
            transactions_status: HashMap::new(),
            blocks: HashMap::new(),
            blocks_meta: HashMap::new(),
            entry: HashMap::new(),
            commitment: Some(CommitmentLevel::Confirmed as i32),
            accounts_data_slice: vec![],
            ping: None,
        };
        
        info!("发送订阅请求...");
        match client.subscribe_once(request.clone()).await {
            Ok(mut stream) => {
                info!("订阅成功，开始接收数据...");
                
                while let Some(message) = stream.next().await {
                    match message {
                        Ok(msg) => {
                            self.process_message(msg).await;
                        }
                        Err(e) => {
                            error!("消息接收错误: {:?}", e);
                            return Err(anyhow::anyhow!("流错误: {:?}", e));
                        }
                    }
                }
            }
            Err(e) => {
                error!("订阅失败: {:?}", e);
                
                info!("尝试备用订阅方法...");
                match client.subscribe().await {
                    Ok((mut sender, mut receiver)) => {
                        info!("备用订阅成功，发送订阅请求...");
                        
                        if let Err(e) = sender.send(request).await {
                            error!("发送订阅请求失败: {:?}", e);
                            return Err(anyhow::anyhow!("发送订阅请求失败"));
                        }
                        
                        info!("开始接收数据...");
                        
                        while let Some(message) = receiver.next().await {
                            match message {
                                Ok(msg) => {
                                    self.process_message(msg).await;
                                }
                                Err(e) => {
                                    error!("消息接收错误: {:?}", e);
                                    return Err(anyhow::anyhow!("流错误: {:?}", e));
                                }
                            }
                        }
                    }
                    Err(e2) => {
                        error!("备用订阅也失败: {:?}", e2);
                        return Err(anyhow::anyhow!("所有订阅方法都失败"));
                    }
                }
            }
        }
        
        Ok(())
    }

    async fn process_message(&self, msg: SubscribeUpdate) {
        if let Some(update_oneof) = &msg.update_oneof {
            use yellowstone_grpc_proto::geyser::subscribe_update::UpdateOneof;
            
            match update_oneof {
                UpdateOneof::Transaction(tx_update) => {
                    self.process_transaction(tx_update);
                }
                UpdateOneof::Account(account) => {
                    if let Some(acc) = &account.account {
                        let sol = acc.lamports as f64 / 1_000_000_000.0;
                        info!("=== 账户更新 ===");
                        info!("余额: {} SOL", sol);
                    }
                }
                UpdateOneof::Ping(_) => {
                    // 忽略ping消息
                }
                _ => {
                    // 忽略其他更新
                }
            }
        }
    }

    fn process_transaction(&self, tx_update: &SubscribeUpdateTransaction) {
        if let Some(tx_info) = &tx_update.transaction {
            // 获取签名
            let signature = bs58::encode(&tx_info.signature).into_string();
            
            info!("╔════════════════ 🔄 检测到新交易 ════════════════╗");
            info!("║ 签名: {}...{}", &signature[..8], &signature[signature.len()-8..]);
            info!("║ 链接: https://solscan.io/tx/{}", signature);
            
            // 检查是否有实际的交易数据和元数据
            if let (Some(transaction), Some(meta)) = (&tx_info.transaction, &tx_info.meta) {
                if let Some(message) = &transaction.message {
                    // 转换账户键为字符串列表
                    let account_keys: Vec<String> = message.account_keys.iter()
                        .map(|key| bs58::encode(key).into_string())
                        .collect();
                    
                    // 标记是否找到了DEX交易
                    let mut found_dex_trade = false;
                    
                    // 查找包含实际指令数据的交易指令
                    for (instruction_index, instruction) in message.instructions.iter().enumerate() {
                        // 获取程序ID
                        let program_id = if (instruction.program_id_index as usize) < account_keys.len() {
                            &account_keys[instruction.program_id_index as usize]
                        } else {
                            continue;
                        };
                        
                        // 检查是否为我们关注的DEX
                        if program_id != crate::types::RAYDIUM_AMM_V4 && 
                           program_id != crate::types::RAYDIUM_CPMM &&
                           program_id != crate::types::RAYDIUM_CLMM &&
                           program_id != crate::types::PUMP_FUN_PROGRAM {
                            continue;
                        }
                        
                        found_dex_trade = true;
                        info!("║ 检测到DEX交易，程序ID: {}", program_id);
                        info!("║ 指令索引: {}", instruction_index);
                        
                        // 准备代币余额数据（需要转换为serde_json::Value）
                        let pre_token_balances: Vec<serde_json::Value> = meta.pre_token_balances.iter()
                            .map(|balance| {
                                serde_json::json!({
                                    "accountIndex": balance.account_index,
                                    "mint": balance.mint,
                                    "owner": balance.owner,
                                    "programId": balance.program_id,
                                    "uiTokenAmount": {
                                        "amount": balance.ui_token_amount.as_ref().map(|ui| &ui.amount).unwrap_or(&"0".to_string()),
                                        "decimals": balance.ui_token_amount.as_ref().map(|ui| ui.decimals).unwrap_or(0),
                                        "uiAmountString": balance.ui_token_amount.as_ref().map(|ui| &ui.ui_amount_string).unwrap_or(&"0".to_string())
                                    }
                                })
                            })
                            .collect();
                        
                        let post_token_balances: Vec<serde_json::Value> = meta.post_token_balances.iter()
                            .map(|balance| {
                                serde_json::json!({
                                    "accountIndex": balance.account_index,
                                    "mint": balance.mint,
                                    "owner": balance.owner,
                                    "programId": balance.program_id,
                                    "uiTokenAmount": {
                                        "amount": balance.ui_token_amount.as_ref().map(|ui| &ui.amount).unwrap_or(&"0".to_string()),
                                        "decimals": balance.ui_token_amount.as_ref().map(|ui| ui.decimals).unwrap_or(0),
                                        "uiAmountString": balance.ui_token_amount.as_ref().map(|ui| &ui.ui_amount_string).unwrap_or(&"0".to_string())
                                    }
                                })
                            })
                            .collect();
                        
                        // 创建解析器并解析交易
                        let parser = TransactionParser::new();
                        info!("║ 开始解析交易指令...");
                        
                        match parser.parse_transaction_data(
                            &signature,
                            &account_keys,
                            &instruction.data,
                            &meta.pre_balances,
                            &meta.post_balances,
                            &pre_token_balances,
                            &post_token_balances,
                            &meta.log_messages,
                        ) {
                            Ok(Some(trade_details)) => {
                                info!("║ ✅ 交易解析成功！");
                                self.handle_parsed_trade(trade_details);
                            }
                            Ok(None) => {
                                info!("║ ⚠️  交易不是swap操作，跳过");
                            }
                            Err(e) => {
                                error!("║ ❌ 解析交易失败: {}", e);
                                // 打印更多调试信息
                                debug!("║ 指令数据长度: {}", instruction.data.len());
                                if instruction.data.len() > 0 {
                                    debug!("║ 指令类型: 0x{:02x}", instruction.data[0]);
                                }
                            }
                        }
                    }
                    
                    // 如果没有找到DEX交易，仍然显示基本信息
                    if !found_dex_trade {
                        // 保留原有的余额分析逻辑
                        if let Some(dex_name) = self.identify_dex(transaction) {
                            info!("║ DEX平台: {}", dex_name);
                        }
                        
                        let fee_sol = meta.fee as f64 / 1_000_000_000.0;
                        info!("║ Gas费: {} SOL", fee_sol);
                        
                        self.analyze_balance_changes(meta, &transaction.message);
                    }
                }
            }
            
            info!("╚═══════════════════════════════════════════════╝");
        }
    }
    
    /// 处理解析后的交易
    fn handle_parsed_trade(&self, trade: TradeDetails) {
        info!("╔═══════════════ 📊 交易解析成功 ═══════════════╗");
        info!("║ DEX平台: {:?}", trade.dex_type);
        info!("║ 交易方向: {:?}", trade.trade_direction);
        info!("║ 交易钱包: {}", trade.wallet);
        info!("║ 代币对: {} -> {}", 
            trade.token_in.symbol.as_ref().unwrap_or(&format!("代币({}...{})", 
                &trade.token_in.mint.to_string()[..4],
                &trade.token_in.mint.to_string().chars().rev().take(4).collect::<String>().chars().rev().collect::<String>()
            )),
            trade.token_out.symbol.as_ref().unwrap_or(&format!("代币({}...{})",
                &trade.token_out.mint.to_string()[..4],
                &trade.token_out.mint.to_string().chars().rev().take(4).collect::<String>().chars().rev().collect::<String>()
            ))
        );
        info!("║ 输入金额: {} {}",
            self.format_token_amount(trade.amount_in, trade.token_in.decimals),
            trade.token_in.symbol.as_ref().unwrap_or(&"代币".to_string())
        );
        info!("║ 输出金额: {} {}",
            self.format_token_amount(trade.amount_out, trade.token_out.decimals),
            trade.token_out.symbol.as_ref().unwrap_or(&"代币".to_string())
        );
        info!("║ 价格: {:.8} SOL/代币", trade.price);
        info!("║ 池子地址: {}", trade.pool_address);
        info!("║ Gas费用: {:.6} SOL", trade.gas_fee as f64 / 1e9);
        info!("╚════════════════════════════════════════════╝");
        
        // 检查是否为目标钱包的交易
        if trade.wallet == self.target_wallet {
            self.handle_target_wallet_trade(trade);
        } else {
            info!("⚠️  交易不是来自目标钱包，跳过跟单");
        }
    }

    /// 处理目标钱包的交易
    fn handle_target_wallet_trade(&self, trade: TradeDetails) {
        info!("🎯 检测到目标钱包交易！准备分析是否跟单...");
        
        // 显示交易摘要
        match trade.trade_direction {
            crate::types::TradeDirection::Buy => {
                info!("💰 目标钱包买入操作:");
                info!("   使用 {} SOL", self.format_token_amount(trade.amount_in, 9));
                info!("   买入 {} {}", 
                    self.format_token_amount(trade.amount_out, trade.token_out.decimals),
                    trade.token_out.symbol.as_ref().unwrap_or(&"未知代币".to_string())
                );
                info!("   代币地址: {}", trade.token_out.mint);
            }
            crate::types::TradeDirection::Sell => {
                info!("💸 目标钱包卖出操作:");
                info!("   卖出 {} {}", 
                    self.format_token_amount(trade.amount_in, trade.token_in.decimals),
                    trade.token_in.symbol.as_ref().unwrap_or(&"未知代币".to_string())
                );
                info!("   获得 {} SOL", self.format_token_amount(trade.amount_out, 9));
                info!("   代币地址: {}", trade.token_in.mint);
            }
        }
        
        // TODO: 在这里连接到第四阶段的决策系统
        // 目前只是记录交易信息，后续将添加：
        // 1. 风险评估
        // 2. 仓位计算
        // 3. 跟单决策
        // 4. 交易执行
        
        info!("📝 交易已记录，等待决策系统实现...");
        
        // 可以将交易保存到文件或数据库以便后续分析
        self.save_trade_for_analysis(&trade);
    }

    /// 保存交易数据以供分析
    fn save_trade_for_analysis(&self, trade: &TradeDetails) {
        // 创建交易记录文件
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let filename = format!("trades/trade_{}_{}.json", timestamp, &trade.signature[..8]);
        
        match serde_json::to_string_pretty(trade) {
            Ok(json_str) => {
                match std::fs::create_dir_all("trades") {
                    Ok(_) => {
                        match std::fs::write(&filename, json_str) {
                            Ok(_) => info!("交易数据已保存到: {}", filename),
                            Err(e) => error!("保存交易数据失败: {}", e),
                        }
                    }
                    Err(e) => error!("创建目录失败: {}", e),
                }
            }
            Err(e) => error!("序列化交易数据失败: {}", e),
        }
    }

    /// 格式化代币数量（改进版）
    fn format_token_amount(&self, amount: u64, decimals: u8) -> String {
        let divisor = 10f64.powi(decimals as i32);
        let value = amount as f64 / divisor;
        
        // 根据数值大小选择合适的显示格式
        if value == 0.0 {
            "0".to_string()
        } else if value < 0.00001 {
            format!("{:.2e}", value)  // 科学计数法
        } else if value < 0.01 {
            format!("{:.6}", value)
        } else if value < 1.0 {
            format!("{:.4}", value)
        } else if value < 1000.0 {
            format!("{:.2}", value)
        } else if value < 1_000_000.0 {
            format!("{:.0}", value)
        } else {
            format!("{:.2}M", value / 1_000_000.0)
        }
    }

    fn identify_dex(&self, transaction: &Transaction) -> Option<String> {
        if let Some(message) = &transaction.message {
            for account_key in &message.account_keys {
                let key_str = bs58::encode(account_key).into_string();
                
                if key_str == RAYDIUM_V4 {
                    return Some("Raydium V4".to_string());
                } else if key_str == JUPITER_V6 {
                    return Some("Jupiter V6".to_string());
                } else if key_str == ORCA_WHIRLPOOL {
                    return Some("Orca Whirlpool".to_string());
                }
            }
        }
        None
    }

    fn analyze_balance_changes(&self, meta: &TransactionStatusMeta, message: &Option<Message>) {
        if meta.pre_balances.len() > 0 && meta.post_balances.len() > 0 {
            info!("║ ---- 余额变化分析 ----");
            
            let account_keys = message.as_ref()
                .map(|m| &m.account_keys)
                .map(|keys| keys.iter()
                    .map(|k| bs58::encode(k).into_string())
                    .collect::<Vec<String>>())
                .unwrap_or_default();
            
            for (i, (pre, post)) in meta.pre_balances.iter()
                .zip(meta.post_balances.iter()).enumerate() {
                if pre != post {
                    let change = *post as i64 - *pre as i64;
                    let change_sol = change as f64 / 1_000_000_000.0;
                    
                    if change_sol.abs() > 0.0001 {
                        let account_str = if i < account_keys.len() {
                            let addr = &account_keys[i];
                            if *addr == self.target_wallet.to_string() {
                                format!("目标钱包")
                            } else if addr == "So11111111111111111111111111111111111111112" {
                                format!("SOL")
                            } else {
                                format!("{}...{}", &addr[..4], &addr[addr.len()-4..])
                            }
                        } else {
                            format!("账户 {}", i)
                        };
                        
                        if change > 0 {
                            info!("║ {} 收到: +{:.6} SOL", account_str, change_sol);
                        } else {
                            info!("║ {} 发送: {:.6} SOL", account_str, change_sol);
                        }
                    }
                }
            }
            
            if meta.pre_token_balances.len() > 0 || meta.post_token_balances.len() > 0 {
                info!("║ ---- 代币余额变化 ----");
                self.analyze_token_balance_changes(meta);
            }
        }
    }

    fn analyze_token_balance_changes(&self, meta: &TransactionStatusMeta) {
        let mut token_changes: HashMap<usize, (Option<u64>, Option<u64>, Option<String>)> = HashMap::new();
        
        for pre_balance in &meta.pre_token_balances {
            let key = pre_balance.account_index as usize;
            let amount = pre_balance.ui_token_amount.as_ref()
                .and_then(|ui| ui.ui_amount_string.parse::<f64>().ok())
                .map(|v| (v * 10f64.powi(pre_balance.ui_token_amount.as_ref().map(|ui| ui.decimals).unwrap_or(0) as i32)) as u64);
            token_changes.entry(key).or_insert((None, None, None)).0 = amount;
            token_changes.entry(key).or_insert((None, None, None)).2 = Some(pre_balance.mint.clone());
        }
        
        for post_balance in &meta.post_token_balances {
            let key = post_balance.account_index as usize;
            let amount = post_balance.ui_token_amount.as_ref()
                .and_then(|ui| ui.ui_amount_string.parse::<f64>().ok())
                .map(|v| (v * 10f64.powi(post_balance.ui_token_amount.as_ref().map(|ui| ui.decimals).unwrap_or(0) as i32)) as u64);
            token_changes.entry(key).or_insert((None, None, None)).1 = amount;
            if token_changes.get(&key).unwrap().2.is_none() {
                token_changes.entry(key).or_insert((None, None, None)).2 = Some(post_balance.mint.clone());
            }
        }
        
        for (_account_index, (pre, post, mint)) in token_changes {
            if let (Some(pre_amount), Some(post_amount), Some(mint_addr)) = (pre, post, mint) {
                if pre_amount != post_amount {
                    let change = post_amount as i64 - pre_amount as i64;
                    let token_symbol = self.get_token_symbol(&mint_addr);
                    
                    if change > 0 {
                        info!("║ 代币收到: +{} {} ({}...{})", 
                            change, token_symbol, &mint_addr[..4], &mint_addr[mint_addr.len()-4..]);
                    } else {
                        info!("║ 代币发送: {} {} ({}...{})", 
                            change.abs(), token_symbol, &mint_addr[..4], &mint_addr[mint_addr.len()-4..]);
                    }
                }
            }
        }
    }

    fn get_token_symbol(&self, mint: &str) -> String {
        match mint {
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" => "USDC".to_string(),
            "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB" => "USDT".to_string(),
            _ => "未知".to_string(),
        }
    }
}