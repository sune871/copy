use anyhow::{Result, Context};
use futures::{StreamExt, SinkExt};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::bs58;
use solana_sdk::signature::Signer;
use std::collections::HashMap;
use std::collections::HashSet;
use tracing::{info, error, warn};
use yellowstone_grpc_client::GeyserGrpcClient;
use yellowstone_grpc_proto::geyser::{
    CommitmentLevel, SubscribeRequest, SubscribeRequestFilterAccounts,
    SubscribeRequestFilterTransactions, SubscribeUpdate, SubscribeUpdateTransaction,
};
use yellowstone_grpc_proto::prelude::{Transaction, Message, TransactionStatusMeta};

// 添加新的导入
use crate::parser::TransactionParser;
use crate::types::TradeDetails;
use crate::trade_executor::{TradeExecutor, PumpFunAccounts, RaydiumCpmmSwapAccounts};
use crate::trade_recorder::TradeRecorder;
use serde_json;
use std::str::FromStr;
use std::sync::Arc;

// Common DEX program IDs
const RAYDIUM_V4: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";
const JUPITER_V6: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";
const ORCA_WHIRLPOOL: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";

// 移除#[derive(Clone)]
pub struct GrpcMonitor {
    endpoint: String,
    auth_token: Option<String>,
    target_wallet: Pubkey,
    executor: Option<Arc<TradeExecutor>>,
    recorder: Option<TradeRecorder>,
    // 在GrpcMonitor结构体中添加已处理指令集合
    processed_instructions: std::sync::Mutex<HashSet<(String, usize)>>,
}

impl GrpcMonitor {
    pub fn new(endpoint: String, auth_token: Option<String>, target_wallet: Pubkey) -> Self {
        GrpcMonitor {
            endpoint,
            auth_token,
            target_wallet,
            executor: None,
            recorder: None,
            processed_instructions: std::sync::Mutex::new(HashSet::new()),
        }
    }
    
    pub fn new_with_executor(
        endpoint: String, 
        auth_token: Option<String>, 
        target_wallet: Pubkey,
        executor: Arc<TradeExecutor>,
    ) -> Self {
        GrpcMonitor {
            endpoint,
            auth_token,
            target_wallet,
            executor: Some(executor),
            recorder: None,
            processed_instructions: std::sync::Mutex::new(HashSet::new()),
        }
    }
    
    pub fn new_with_executor_and_recorder(
        endpoint: String, 
        auth_token: Option<String>, 
        target_wallet: Pubkey,
        executor: Arc<TradeExecutor>,
        recorder: TradeRecorder,
    ) -> Self {
        GrpcMonitor {
            endpoint,
            auth_token,
            target_wallet,
            executor: Some(executor),
            recorder: Some(recorder),
            processed_instructions: std::sync::Mutex::new(HashSet::new()),
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
                    self.process_transaction(tx_update).await;
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

    async fn process_transaction(&self, tx_update: &SubscribeUpdateTransaction) {
        if let Some(tx_info) = &tx_update.transaction {
            // 获取签名
            let signature = bs58::encode(&tx_info.signature).into_string();
            if let (Some(transaction), Some(meta)) = (&tx_info.transaction, &tx_info.meta) {
                if let Some(message) = &transaction.message {
                    let account_keys: Vec<String> = message.account_keys.iter()
                        .map(|key| bs58::encode(key).into_string())
                        .collect();
                    let mut found_dex_trade = false;
                    let mut is_pump_trade = false;
                    for (instruction_index, instruction) in message.instructions.iter().enumerate() {
                        let program_id = if (instruction.program_id_index as usize) < account_keys.len() {
                            &account_keys[instruction.program_id_index as usize]
                        } else {
                            continue;
                        };
                        if program_id != crate::types::RAYDIUM_AMM_V4 && 
                           program_id != crate::types::RAYDIUM_CPMM &&
                           program_id != crate::types::RAYDIUM_CLMM &&
                           program_id != crate::types::PUMP_FUN_PROGRAM {
                            continue;
                        }
                        if program_id == crate::types::PUMP_FUN_PROGRAM {
                            is_pump_trade = true;
                        }
                        found_dex_trade = true;
                        // 去重：同一signature+指令索引只处理一次
                        let mut processed = self.processed_instructions.lock().unwrap();
                        if processed.contains(&(signature.clone(), instruction_index)) {
                            continue;
                        }
                        processed.insert((signature.clone(), instruction_index));
                        drop(processed);
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
                        let parser = TransactionParser::new();
                        let trade_result = parser.parse_transaction_data(
                            &signature,
                            &account_keys,
                            &instruction.data,
                            &meta.pre_balances,
                            &meta.post_balances,
                            &pre_token_balances,
                            &post_token_balances,
                            &meta.log_messages,
                        );
                        match trade_result {
                            Ok(Some(trade_details)) => {
                                self.handle_parsed_trade(trade_details, account_keys.clone());
                                found_dex_trade = true;
                            }
                            Ok(None) => {}
                            Err(e) => {
                                warn!("解析交易失败: {}", e);
                            }
                        }
                    }
                    if !found_dex_trade {
                        if let Some(dex_name) = self.identify_dex(transaction) {
                            info!("║ DEX平台: {}", dex_name);
                        }
                        let fee_sol = meta.fee as f64 / 1_000_000_000.0;
                        info!("║ Gas费: {} SOL", fee_sol);
                        if !is_pump_trade {
                            self.analyze_balance_changes(meta, &transaction.message);
                        } else {
                            info!("║ [Pump提示] 该交易为Pump.fun，已省略详细余额变化分析，仅看上方业务摘要即可");
                        }
                    }
                }
            }
        }
    }

    /// 处理解析后的交易和账户
    fn handle_parsed_trade(&self, trade: TradeDetails, account_keys: Vec<String>) {
        info!("[DEBUG] trade.wallet = {}, self.target_wallet = {}", trade.wallet, self.target_wallet);
        info!("[DEBUG] 相等判断: {}", trade.wallet == self.target_wallet);
        if trade.dex_type == crate::types::DexType::PumpFun {
            info!("╔═══════════════ 📊 Pump.fun 交易解析 ═══════════════╗");
            info!("║ DEX平台: Pump.fun");
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
            info!("║ [Pump提示] 该交易链上会有mint/销毁/分账等多种Token流转，以下只展示用户实际swap的输入输出");
            info!("╚════════════════════════════════════════════╝");
        } else {
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
            info!("║ 输入金额: {}",
                self.format_token_amount(trade.amount_in, trade.token_in.decimals)
            );
            info!("║ 输出金额: {} {}",
                self.format_token_amount(trade.amount_out, trade.token_out.decimals),
                trade.token_out.symbol.as_ref().unwrap_or(&"代币".to_string())
            );
            info!("║ 价格: {:.8} SOL/代币", trade.price);
            info!("║ 池子地址: {}", trade.pool_address);
            info!("║ Gas费用: {:.6} SOL", trade.gas_fee as f64 / 1e9);
            info!("╚════════════════════════════════════════════╝");
        }
        if trade.wallet == self.target_wallet {
            info!("[DEBUG] 进入目标钱包跟单分支");
            if let Some(executor) = &self.executor {
                info!("[DEBUG] executor已配置，准备执行跟单");
                let executor = Arc::clone(executor);
                match trade.dex_type {
                    crate::types::DexType::RaydiumCPMM => {
                        // 以链上TX顺序组装cpmm_accounts和extra_accounts
                        if account_keys.len() >= 16 {
                            info!("[DEBUG] Raydium CPMM分支，account_keys数量: {}", account_keys.len());
                            let cpmm_accounts = RaydiumCpmmSwapAccounts {
                                payer: Pubkey::from_str(&account_keys[0]).unwrap(),
                                user_input_ata: Pubkey::from_str(&account_keys[1]).unwrap(),
                                user_output_ata: Pubkey::from_str(&account_keys[2]).unwrap(),
                                pool_state: Pubkey::from_str(&account_keys[3]).unwrap(),
                                authority: Pubkey::from_str(&account_keys[4]).unwrap(),
                                amm_config: Pubkey::from_str(&account_keys[5]).unwrap(),
                                observation_state: Pubkey::from_str(&account_keys[6]).unwrap(),
                                input_vault: Pubkey::from_str(&account_keys[7]).unwrap(),
                                output_vault: Pubkey::from_str(&account_keys[8]).unwrap(),
                                input_token_program: Pubkey::from_str(&account_keys[9]).unwrap(),
                                output_token_program: Pubkey::from_str(&account_keys[10]).unwrap(),
                                input_mint: Pubkey::from_str(&account_keys[11]).unwrap(),
                                output_mint: Pubkey::from_str(&account_keys[12]).unwrap(),
                            };
                            let extra_accounts = account_keys[13..].iter().map(|k| Pubkey::from_str(k).unwrap()).collect::<Vec<_>>();
                            let min_amount_out = (trade.amount_out as f64 * (1.0 - executor.config.slippage_tolerance)) as u64;
                            let trade_clone = trade.clone();
                            let cpmm_accounts_clone = cpmm_accounts.clone();
                            let extra_accounts_clone = extra_accounts.clone();
                            let executor = Arc::clone(&executor);
                            let wallet = executor.copy_wallet.clone();
                            let rpc_url = executor.rpc_url.clone();
                            tokio::spawn(async move {
                                let client = solana_client::rpc_client::RpcClient::new(rpc_url);
                                info!("[DEBUG] tokio::spawn内，先同步创建ATA");
                                if let Err(e) = TradeExecutor::ensure_ata_exists_static(&client, &wallet, &wallet.pubkey(), &trade_clone.token_in.mint) {
                                    warn!("[ATA] 创建token_in ATA失败: {}", e);
                                    return;
                                }
                                if let Err(e) = TradeExecutor::ensure_ata_exists_static(&client, &wallet, &wallet.pubkey(), &trade_clone.token_out.mint) {
                                    warn!("[ATA] 创建token_out ATA失败: {}", e);
                                    return;
                                }
                                info!("[DEBUG] ATA已全部创建，开始执行swap跟单");
                                let res = TradeExecutor::execute_raydium_cpmm_trade_static(&client, &wallet, &trade_clone, &cpmm_accounts_clone, &extra_accounts_clone, min_amount_out).await;
                                info!("[DEBUG] 跟单执行结果: {:?}", res);
                            });
                        } else {
                            warn!("[DEBUG] Raydium CPMM分支，account_keys数量不足，跳过跟单，当前keys: {:?}", account_keys);
                        }
                    }
                    crate::types::DexType::PumpFun => {
                        if account_keys.len() >= 11 {
                            info!("[DEBUG] PumpFun分支，account_keys数量: {}", account_keys.len());
                            let pump_accounts = PumpFunAccounts {
                                fee_recipient: Pubkey::from_str(&account_keys[1]).unwrap(),
                                mint: Pubkey::from_str(&account_keys[2]).unwrap(),
                                bonding_curve: Pubkey::from_str(&account_keys[3]).unwrap(),
                                associated_bonding_curve: Pubkey::from_str(&account_keys[4]).unwrap(),
                                event_authority: Pubkey::from_str(&account_keys[10]).unwrap(),
                            };
                            let max_sol_cost = trade.amount_in;
                            let trade_clone = trade.clone();
                            let pump_accounts_clone = pump_accounts.clone();
                            info!("[DEBUG] 跟单参数: max_sol_cost={}", max_sol_cost);
                            tokio::spawn(async move {
                                info!("[DEBUG] tokio::spawn内，开始创建Pump指令");
                                let _ = executor.create_pump_instructions(&trade_clone, &pump_accounts_clone, max_sol_cost);
                                info!("[DEBUG] tokio::spawn内，开始执行Pump跟单");
                                let res = executor.execute_trade(&trade_clone).await;
                                info!("[DEBUG] 跟单执行结果: {:?}", res);
                            });
                        } else {
                            warn!("[DEBUG] PumpFun分支，account_keys数量不足，跳过跟单");
                        }
                    }
                    _ => {
                        warn!("[DEBUG] 未知DEX类型，跳过跟单");
                    }
                }
            } else {
                warn!("[DEBUG] executor未配置，无法跟单");
            }
        } else {
            info!("[DEBUG] 交易不是目标钱包，跳过跟单");
        }
        self.save_trade_for_analysis(&trade);
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
        
        // 执行跟单交易
        if let Some(_executor) = &self.executor {
            info!("🚀 开始执行跟单交易...");
            
            // 由于TradeExecutor不支持Clone，我们需要在这里直接执行
            // 注意：这可能会阻塞监控线程，在生产环境中应该使用更好的异步处理方式
            let _trade_clone = trade.clone();
            
            // 使用tokio::spawn在后台执行交易
            tokio::spawn(async move {
                // 这里我们需要重新创建TradeExecutor实例
                // 在实际应用中，应该使用更好的架构来处理这个问题
                warn!("⚠️  跟单功能需要重新实现以支持异步执行");
            });
        } else {
            info!("⚠️  交易执行器未配置，跳过跟单");
        }
        
        // 保存交易记录
        self.save_trade_for_analysis(&trade);
    }

    /// 保存交易数据以供分析
    fn save_trade_for_analysis(&self, trade: &TradeDetails) {
        // 使用交易记录器保存交易
        if let Some(recorder) = &self.recorder {
            if let Err(e) = recorder.record_trade(trade) {
                error!("保存交易记录失败: {}", e);
            }
        } else {
            info!("交易记录器未配置，跳过保存");
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
        // 检查是否为PumpFun类型交易，如果是则跳过详细余额变化分析
        if let Some(msg) = message {
            // 取出所有account_keys
            let account_keys: Vec<String> = msg.account_keys.iter()
                .map(|k| bs58::encode(k).into_string())
                .collect();
            // 判断是否包含PumpFun program id
            if account_keys.iter().any(|k| k == crate::types::PUMP_FUN_PROGRAM) {
                info!("║ [Pump提示] 该交易为Pump.fun，已省略详细余额变化分析，仅看上方业务摘要即可");
                return;
            }
        }
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