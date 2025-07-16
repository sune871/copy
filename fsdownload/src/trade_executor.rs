use anyhow::{Result, Context};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    instruction::Instruction,
    message::Message,
    signature::Keypair,
    signer::Signer,
    transaction::Transaction,
};
use tracing::{info, warn, error};
use crate::types::{TradeDetails, TradeDirection, TradeExecutionConfig, ExecutedTrade, DexType};
use chrono::Utc;
use solana_sdk::pubkey::Pubkey;
use spl_associated_token_account::get_associated_token_address;
use solana_sdk::instruction::AccountMeta;
use solana_account_decoder::UiAccountData;
use std::str::FromStr;
use solana_client::rpc_request::TokenAccountsFilter;
// 不再引入solana_account_decoder，直接用solana_client::rpc_response::UiAccountData
use std::sync::Arc;

// Raydium池子账户结构体
#[derive(Clone)]
pub struct RaydiumPoolAccounts {
    pub amm_id: Pubkey,
    pub amm_authority: Pubkey,
    pub amm_open_orders: Pubkey,
    pub amm_target_orders: Pubkey,
    pub pool_coin_token_account: Pubkey,
    pub pool_pc_token_account: Pubkey,
    pub serum_program_id: Pubkey,
    pub serum_market: Pubkey,
    pub serum_bids: Pubkey,
    pub serum_asks: Pubkey,
    pub serum_event_queue: Pubkey,
    pub serum_coin_vault_account: Pubkey,
    pub serum_pc_vault_account: Pubkey,
    pub serum_vault_signer: Pubkey,
}

// Pump.fun账户结构体
#[derive(Clone)]
pub struct PumpFunAccounts {
    pub fee_recipient: Pubkey,
    pub mint: Pubkey,
    pub bonding_curve: Pubkey,
    pub associated_bonding_curve: Pubkey,
    pub event_authority: Pubkey,
}

// Raydium CPMM swap指令账户结构体
#[derive(Clone, Debug)]
pub struct RaydiumCpmmSwapAccounts {
    pub payer: Pubkey,
    pub authority: Pubkey,
    pub amm_config: Pubkey,
    pub pool_state: Pubkey,
    pub user_input_ata: Pubkey,
    pub user_output_ata: Pubkey,
    pub input_vault: Pubkey,
    pub output_vault: Pubkey,
    pub input_token_program: Pubkey,
    pub output_token_program: Pubkey,
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub observation_state: Pubkey,
}

pub struct TradeExecutor {
    pub client: RpcClient,
    pub copy_wallet: Arc<Keypair>,
    pub config: TradeExecutionConfig,
    pub rpc_url: String, // 新增
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
        
        let copy_wallet = Arc::new(Keypair::from_bytes(&private_key_bytes)
            .context("无法从私钥创建钱包")?);
        
        info!("交易执行器初始化完成，钱包地址: {}", copy_wallet.pubkey());
        
        Ok(TradeExecutor {
            client,
            copy_wallet,
            config,
            rpc_url: rpc_url.to_string(), // 新增
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
        // 检查是否强制下单金额
        let mut trade_amount_sol = trade.amount_in as f64 / 1_000_000_000.0;
        let mut forced = false;
        if (self.config.max_trade_amount - self.config.min_trade_amount).abs() < 1e-9 {
            trade_amount_sol = self.config.max_trade_amount;
            forced = true;
        }
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
        if trade_amount_sol > self.config.max_trade_amount && !forced {
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
        if forced {
            info!("[强制下单] 按配置金额下单: {} SOL (原链上amount_in: {:.6} SOL)", trade_amount_sol, trade.amount_in as f64 / 1_000_000_000.0);
        }
        // ====== 卖出前自动检测copy钱包目标币种余额 ======
        if trade.trade_direction == TradeDirection::Sell {
            let token_mint = trade.token_in.mint;
            let token_accounts = self.client.get_token_accounts_by_owner(
                &self.copy_wallet.pubkey(),
                TokenAccountsFilter::Mint(token_mint),
            )?;
            let mut total_token_balance = 0u64;
            for acc in token_accounts {
                if let UiAccountData::Json(value) = &acc.account.data {
                    // 1.17/1.18的Json变体是ParsedAccount结构，不是serde_json::Value
                    // 需要访问value.parsed字段（通常是serde_json::Value），再取info
                    if let Some(info) = value.parsed.get("info") {
                        if let Some(token_amount) = info.get("tokenAmount")
                            .and_then(|ta| ta.get("amount"))
                            .and_then(|a| a.as_str())
                            .and_then(|s| s.parse::<u64>().ok()) {
                            total_token_balance += token_amount;
                        }
                    }
                }
            }
            if total_token_balance < trade_forced_amount_in_lamports(trade_amount_sol) {
                warn!("[风控] 跟单钱包无足够{}余额，跳过卖出。余额: {}，需卖出: {}", trade.token_in.symbol.as_ref().unwrap_or(&"目标币种".to_string()), total_token_balance, trade_forced_amount_in_lamports(trade_amount_sol));
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
                    error_message: Some("跟单钱包无该币种余额，跳过卖出".to_string()),
                });
            }
        }
        // ====== 买入/卖出需要WSOL时自动检测并兑换 ======
        let wsol_mint = Pubkey::from_str(crate::types::WSOL_MINT).unwrap();
        let need_wsol = (trade.trade_direction == TradeDirection::Buy && trade.token_in.mint == wsol_mint)
            || (trade.trade_direction == TradeDirection::Sell && trade.token_out.mint == wsol_mint);
        if need_wsol {
            let wsol_ata = get_associated_token_address(&self.copy_wallet.pubkey(), &wsol_mint);
            let wsol_balance = self.client.get_token_account_balance(&wsol_ata).map(|b| b.amount.parse::<u64>().unwrap_or(0)).unwrap_or(0);
            let required = trade_forced_amount_in_lamports(trade_amount_sol);
            if wsol_balance < required {
                let sol_balance = self.client.get_balance(&self.copy_wallet.pubkey())?;
                if sol_balance < required {
                    warn!("[风控] SOL余额不足，无法自动兑换WSOL。SOL余额: {}，需兑换: {}", sol_balance, required);
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
                        error_message: Some("SOL余额不足，无法自动兑换WSOL".to_string()),
                    });
                }
                info!("[自动兑换] 正在将SOL兑换为WSOL，金额: {} lamports", required - wsol_balance);
                // 创建WSOL账户（ATA）
                let create_ata_ix = spl_associated_token_account::instruction::create_associated_token_account(
                    &self.copy_wallet.pubkey(),
                    &self.copy_wallet.pubkey(),
                    &wsol_mint,
                    &spl_token::id(),
                );
                // 转账SOL到WSOL账户
                let transfer_ix = solana_sdk::system_instruction::transfer(
                    &self.copy_wallet.pubkey(),
                    &wsol_ata,
                    required - wsol_balance,
                );
                // 同步WSOL账户余额
                let sync_ix = spl_token::instruction::sync_native(&spl_token::id(), &wsol_ata)?;
                let message = Message::new(&[create_ata_ix, transfer_ix, sync_ix], Some(&self.copy_wallet.pubkey()));
                let recent_blockhash = self.client.get_latest_blockhash()?;
                let mut tx = Transaction::new_unsigned(message);
                let wallet = self.copy_wallet.clone();
                tx.sign(&[wallet.as_ref()], recent_blockhash);
                self.client.send_and_confirm_transaction(&tx)?;
                info!("[自动兑换] SOL兑换WSOL成功");
            }
        }
        info!("开始执行跟单交易:");
        info!("  原始交易: {}", trade.signature);
        info!("  交易方向: {:?}", trade.trade_direction);
        info!("  交易金额: {:.6} SOL", trade_amount_sol);
        info!("  代币: {:?}", trade.token_out.symbol);
        // 构造一个新的TradeDetails用于实际下单
        let mut trade_for_exec = trade.clone();
        if forced {
            trade_for_exec.amount_in = (trade_amount_sol * 1_000_000_000.0) as u64;
        }
        match trade.dex_type {
            DexType::RaydiumCPMM => {
                warn!("execute_trade已禁用RaydiumCPMM分支，请直接调用execute_raydium_cpmm_trade并传入正确池子参数！");
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
                    error_message: Some("禁止通过execute_trade执行RaydiumCPMM，请用新版接口！".to_string()),
                });
            }
            DexType::PumpFun => {
                self.execute_pump_trade(&trade_for_exec).await
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
    
    /// 自动检查并创建ATA（如不存在）
    pub fn ensure_ata_exists_static(client: &RpcClient, wallet: &Arc<Keypair>, owner: &Pubkey, mint: &Pubkey) -> Result<()> {
        let ata = get_associated_token_address(owner, mint);
        let account = client.get_account_with_commitment(&ata, CommitmentConfig::confirmed())?.value;
        if account.is_none() {
            let ix = spl_associated_token_account::instruction::create_associated_token_account(
                owner, owner, mint, &spl_token::id()
            );
            let message = Message::new(&[ix], Some(owner));
            let recent_blockhash = client.get_latest_blockhash()?;
            let mut tx = Transaction::new_unsigned(message);
            tx.sign(&[wallet.as_ref()], recent_blockhash);
            client.send_and_confirm_transaction(&tx)?;
            info!("[ATA] 已自动创建ATA: {}", ata);
        }
        Ok(())
    }
    
    /// 执行Raydium CPMM交易
    pub async fn execute_raydium_cpmm_trade_static(client: &RpcClient, wallet: &Arc<Keypair>, trade: &TradeDetails, cpmm_accounts: &RaydiumCpmmSwapAccounts, extra_accounts: &[Pubkey], min_amount_out: u64) -> Result<ExecutedTrade> {
        info!("执行Raydium CPMM交易(静态版)...");
        let recent_blockhash = client.get_latest_blockhash()?;
        // 组装swap指令（仍用self的create_raydium_cpmm_swap_instructions_v2，需改为静态或复制逻辑）
        // 这里假设有静态create_raydium_cpmm_swap_instructions_v2
        let instructions = Self::create_raydium_cpmm_swap_instructions_v2_static(trade, cpmm_accounts, extra_accounts, min_amount_out)?;
        let message = Message::new(&instructions, Some(&wallet.pubkey()));
        let mut transaction = Transaction::new_unsigned(message);
        transaction.sign(&[wallet.as_ref()], recent_blockhash);
        match client.send_and_confirm_transaction(&transaction) {
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
        let instructions = self.create_pump_instructions(trade, &PumpFunAccounts {
            fee_recipient: Pubkey::new_from_array([0; 32]),
            mint: Pubkey::new_from_array([0; 32]),
            bonding_curve: Pubkey::new_from_array([0; 32]),
            associated_bonding_curve: Pubkey::new_from_array([0; 32]),
            event_authority: Pubkey::new_from_array([0; 32]),
        }, 0)?;
        
        // 创建交易
        let message = Message::new(&instructions, Some(&self.copy_wallet.pubkey()));
        let mut transaction = Transaction::new_unsigned(message);
        
        // 签名交易
        let wallet = self.copy_wallet.clone();
        transaction.sign(&[wallet.as_ref()], recent_blockhash);
        
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
    pub fn create_raydium_cpmm_instructions(&self, trade: &TradeDetails, pool: &RaydiumPoolAccounts, min_amount_out: u64) -> Result<Vec<Instruction>> {
        let mut instructions = Vec::new();
        let user_pubkey = self.copy_wallet.pubkey();
        let token_in_ata = get_associated_token_address(&user_pubkey, &trade.token_in.mint);
        let token_out_ata = get_associated_token_address(&user_pubkey, &trade.token_out.mint);

        tracing::info!("[DEBUG] 构造Raydium CPMM swap指令账户列表:");
        tracing::info!("user_pubkey: {}", user_pubkey);
        tracing::info!("token_in_ata: {} mint: {}", token_in_ata, trade.token_in.mint);
        tracing::info!("token_out_ata: {} mint: {}", token_out_ata, trade.token_out.mint);
        tracing::info!("池子参数: amm_id={} amm_authority={} amm_open_orders={} amm_target_orders={} pool_coin_token_account={} pool_pc_token_account={} serum_program_id={} serum_market={} serum_bids={} serum_asks={} serum_event_queue={} serum_coin_vault_account={} serum_pc_vault_account={} serum_vault_signer={}",
            pool.amm_id, pool.amm_authority, pool.amm_open_orders, pool.amm_target_orders, pool.pool_coin_token_account, pool.pool_pc_token_account, pool.serum_program_id, pool.serum_market, pool.serum_bids, pool.serum_asks, pool.serum_event_queue, pool.serum_coin_vault_account, pool.serum_pc_vault_account, pool.serum_vault_signer
        );

        // 自动创建ATA（如不存在）
        instructions.push(spl_associated_token_account::instruction::create_associated_token_account(
            &user_pubkey, &user_pubkey, &trade.token_in.mint, &spl_token::id()
        ));
        instructions.push(spl_associated_token_account::instruction::create_associated_token_account(
            &user_pubkey, &user_pubkey, &trade.token_out.mint, &spl_token::id()
        ));

        // 构造Raydium swap指令data
        let mut data = vec![9u8]; // swap指令类型
        data.extend_from_slice(&trade.amount_in.to_le_bytes());
        data.extend_from_slice(&min_amount_out.to_le_bytes());

        // 构造完整账户列表（顺序必须严格按合约要求）
        let accounts = vec![
            AccountMeta::new(user_pubkey, true),
            AccountMeta::new(token_in_ata, false),
            AccountMeta::new(token_out_ata, false),
            AccountMeta::new(pool.amm_id, false),
            AccountMeta::new(pool.amm_authority, false),
            AccountMeta::new(pool.amm_open_orders, false),
            AccountMeta::new(pool.amm_target_orders, false),
            AccountMeta::new(pool.pool_coin_token_account, false),
            AccountMeta::new(pool.pool_pc_token_account, false),
            AccountMeta::new(pool.serum_program_id, false),
            AccountMeta::new(pool.serum_market, false),
            AccountMeta::new(pool.serum_bids, false),
            AccountMeta::new(pool.serum_asks, false),
            AccountMeta::new(pool.serum_event_queue, false),
            AccountMeta::new(pool.serum_coin_vault_account, false),
            AccountMeta::new(pool.serum_pc_vault_account, false),
            AccountMeta::new(pool.serum_vault_signer, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::sysvar::rent::id(), false),
        ];
        for (i, acc) in accounts.iter().enumerate() {
            tracing::info!("账户{}: {} signer:{} writable:{}", i, acc.pubkey, acc.is_signer, acc.is_writable);
        }
        let swap_ix = Instruction {
            program_id: trade.program_id,
            accounts,
            data,
        };
        instructions.push(swap_ix);
        Ok(instructions)
    }

    /// 新版：严格按链上顺序组装Raydium CPMM swap指令
    pub fn create_raydium_cpmm_swap_instructions_v2_static(
        trade: &TradeDetails,
        accounts: &RaydiumCpmmSwapAccounts,
        extra_accounts: &[Pubkey], // 额外serum等账户，按链上顺序
        _min_amount_out: u64, // 未使用变量加下划线
    ) -> Result<Vec<Instruction>> {
        let data = vec![0u8; 16]; // 去除mut
        // 只允许payer为is_signer: true，其余全部为false
        let mut metas = vec![
            AccountMeta::new(accounts.payer, true),
            AccountMeta::new(accounts.user_input_ata, false),
            AccountMeta::new(accounts.user_output_ata, false),
            AccountMeta::new(accounts.pool_state, false),
            AccountMeta::new_readonly(accounts.authority, false),
            AccountMeta::new_readonly(accounts.amm_config, false),
            AccountMeta::new_readonly(accounts.observation_state, false),
            AccountMeta::new(accounts.input_vault, false),
            AccountMeta::new(accounts.output_vault, false),
            AccountMeta::new_readonly(accounts.input_token_program, false),
            AccountMeta::new_readonly(accounts.output_token_program, false),
            AccountMeta::new_readonly(accounts.input_mint, false),
            AccountMeta::new_readonly(accounts.output_mint, false),
        ];
        // extra_accounts全部用AccountMeta::new_readonly
        for pk in extra_accounts {
            metas.push(AccountMeta::new_readonly(*pk, false));
        }
        let swap_ix = Instruction {
            program_id: trade.program_id,
            accounts: metas,
            data,
        };
        Ok(vec![swap_ix])
    }

    pub fn create_pump_instructions(&self, trade: &TradeDetails, accounts: &PumpFunAccounts, max_sol_cost: u64) -> Result<Vec<Instruction>> {
        let mut instructions = Vec::new();
        let user_pubkey = self.copy_wallet.pubkey();
        let token_ata = get_associated_token_address(&user_pubkey, &trade.token_in.mint);

        // 自动创建ATA（如不存在）
        instructions.push(spl_associated_token_account::instruction::create_associated_token_account(
            &user_pubkey, &user_pubkey, &trade.token_in.mint, &spl_token::id()
        ));

        // 构造Pump.fun指令data
        let instruction_type = match trade.trade_direction {
            TradeDirection::Buy => 0x66u8,
            TradeDirection::Sell => 0x33u8,
        };
        let mut data = vec![instruction_type];
        data.extend_from_slice(&trade.amount_in.to_le_bytes());
        data.extend_from_slice(&max_sol_cost.to_le_bytes());

        // 构造完整账户列表
        let accounts_vec = vec![
            AccountMeta::new(user_pubkey, true),
            AccountMeta::new(accounts.fee_recipient, false),
            AccountMeta::new(accounts.mint, false),
            AccountMeta::new(accounts.bonding_curve, false),
            AccountMeta::new(accounts.associated_bonding_curve, false),
            AccountMeta::new(token_ata, false),
            AccountMeta::new(user_pubkey, true),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(solana_sdk::sysvar::rent::id(), false),
            AccountMeta::new(accounts.event_authority, false),
            AccountMeta::new(trade.program_id, false),
        ];
        let pump_ix = Instruction {
            program_id: trade.program_id,
            accounts: accounts_vec,
            data,
        };
        instructions.push(pump_ix);
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

fn trade_forced_amount_in_lamports(trade_amount_sol: f64) -> u64 {
    (trade_amount_sol * 1_000_000_000.0) as u64
} 