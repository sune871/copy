use anyhow::{Result, Context};
use futures::{StreamExt, SinkExt};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::bs58;
use std::collections::HashMap;
use tracing::{info, error, warn};
use yellowstone_grpc_client::GeyserGrpcClient;
use yellowstone_grpc_proto::geyser::{
    CommitmentLevel, SubscribeRequest, SubscribeRequestFilterAccounts,
    SubscribeRequestFilterTransactions, SubscribeUpdate, SubscribeUpdateTransaction,
};
use yellowstone_grpc_proto::prelude::{Transaction, Message, TransactionStatusMeta};

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
        info!("Starting gRPC monitoring service, target wallet: {}", self.target_wallet);
        info!("Connecting to gRPC endpoint: {}", self.endpoint);
        
        loop {
            match self.monitor_loop().await {
                Ok(_) => {
                    warn!("Monitoring loop ended, preparing to restart...");
                }
                Err(e) => {
                    error!("Monitoring error: {:?}", e);
                }
            }
            
            info!("Retrying in 5 seconds...");
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }

    async fn monitor_loop(&self) -> Result<()> {
        let mut client = GeyserGrpcClient::build_from_shared(self.endpoint.clone())?
            .connect()
            .await
            .context("Unable to connect to gRPC service")?;
        
        info!("Connected to gRPC service, preparing to subscribe...");
        
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
        
        info!("Sending subscription request...");
        match client.subscribe_once(request.clone()).await {
            Ok(mut stream) => {
                info!("Subscription successful, starting to receive data...");
                
                while let Some(message) = stream.next().await {
                    match message {
                        Ok(msg) => {
                            self.process_message(msg).await;
                        }
                        Err(e) => {
                            error!("Message reception error: {:?}", e);
                            return Err(anyhow::anyhow!("Stream error: {:?}", e));
                        }
                    }
                }
            }
            Err(e) => {
                error!("Subscription failed: {:?}", e);
                
                info!("Trying alternative subscription method...");
                match client.subscribe().await {
                    Ok((mut sender, mut receiver)) => {
                        info!("Alternative subscription successful, sending subscription request...");
                        
                        if let Err(e) = sender.send(request).await {
                            error!("Failed to send subscription request: {:?}", e);
                            return Err(anyhow::anyhow!("Failed to send subscription request"));
                        }
                        
                        info!("Starting to receive data...");
                        
                        while let Some(message) = receiver.next().await {
                            match message {
                                Ok(msg) => {
                                    self.process_message(msg).await;
                                }
                                Err(e) => {
                                    error!("Message reception error: {:?}", e);
                                    return Err(anyhow::anyhow!("Stream error: {:?}", e));
                                }
                            }
                        }
                    }
                    Err(e2) => {
                        error!("Alternative subscription also failed: {:?}", e2);
                        return Err(anyhow::anyhow!("All subscription methods failed"));
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
                        info!("=== Account Update ===");
                        info!("Balance: {} SOL", sol);
                    }
                }
                UpdateOneof::Ping(_) => {
                    // Ignore ping messages
                }
                _ => {
                    // Ignore other updates
                }
            }
        }
    }

    fn process_transaction(&self, tx_update: &SubscribeUpdateTransaction) {
        if let Some(transaction) = &tx_update.transaction {
            let signature = if transaction.signatures.len() > 0 {
                bs58::encode(&transaction.signatures[0]).into_string()
            } else {
                "Unknown".to_string()
            };
            
            info!("╔════════════════ 🔄 New Transaction Detected ════════════════╗");
            info!("║ Signature: {}...{}", &signature[..8], &signature[signature.len()-8..]);
            info!("║ Link: https://solscan.io/tx/{}", signature);
            
            // Identify DEX
            if let Some(dex_name) = self.identify_dex(transaction) {
                info!("║ DEX Platform: {}", dex_name);
            }
            
            // Display transaction fee and analyze balance changes
            if let Some(meta) = &tx_update.meta {
                let fee_sol = meta.fee as f64 / 1_000_000_000.0;
                info!("║ Gas Fee: {} SOL", fee_sol);
                
                // Analyze balance changes
                self.analyze_balance_changes(meta, &transaction.message);
                
                // Display transaction logs (may contain useful information)
                if meta.log_messages.len() > 0 {
                    info!("║ ---- Transaction Logs ----");
                    for (i, log) in meta.log_messages.iter().enumerate() {
                        if log.contains("Swap") || log.contains("swap") || 
                           log.contains("Buy") || log.contains("Sell") ||
                           log.contains("amount") {
                            info!("║ [{}] {}", i, log);
                        }
                    }
                }
            }
            
            info!("╚═══════════════════════════════════════════════╝");
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
            info!("║ ---- Balance Changes Analysis ----");
            
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
                            if addr == self.target_wallet.to_string() {
                                format!("Target Wallet")
                            } else if addr == "So11111111111111111111111111111111111111112" {
                                format!("SOL")
                            } else {
                                format!("{}...{}", &addr[..4], &addr[addr.len()-4..])
                            }
                        } else {
                            format!("Account {}", i)
                        };
                        
                        if change > 0 {
                            info!("║ {} received: +{:.6} SOL", account_str, change_sol);
                        } else {
                            info!("║ {} sent: {:.6} SOL", account_str, change_sol);
                        }
                    }
                }
            }
            
            if meta.pre_token_balances.len() > 0 || meta.post_token_balances.len() > 0 {
                info!("║ ---- Token Balance Changes ----");
                self.analyze_token_balance_changes(meta);
            }
        }
    }

    fn analyze_token_balance_changes(&self, meta: &TransactionStatusMeta) {
        let mut token_changes: HashMap<usize, (Option<u64>, Option<u64>, Option<String>)> = HashMap::new();
        
        for pre_balance in &meta.pre_token_balances {
            let key = pre_balance.account_index as usize;
            let amount = pre_balance.ui_token_amount.ui_amount_string.parse::<f64>().ok()
                .map(|v| (v * 10f64.powi(pre_balance.ui_token_amount.decimals as i32)) as u64);
            token_changes.entry(key).or_insert((None, None, None)).0 = amount;
            token_changes.entry(key).or_insert((None, None, None)).2 = Some(pre_balance.mint.clone());
        }
        
        for post_balance in &meta.post_token_balances {
            let key = post_balance.account_index as usize;
            let amount = post_balance.ui_token_amount.ui_amount_string.parse::<f64>().ok()
                .map(|v| (v * 10f64.powi(post_balance.ui_token_amount.decimals as i32)) as u64);
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
                        info!("║ Token received: +{} {} ({}...{})", 
                            change, token_symbol, &mint_addr[..4], &mint_addr[mint_addr.len()-4..]);
                    } else {
                        info!("║ Token sent: {} {} ({}...{})", 
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
            _ => "Unknown".to_string(),
        }
    }
}