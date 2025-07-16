use anyhow::{Result, Context};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use tracing::info;
use crate::types::{TradeDetails, DexType, TradeDirection, TokenInfo, WSOL_MINT, RAYDIUM_AMM_SWAP_INSTRUCTION};
use crate::parser;
use chrono::Utc;
use wallet_copier::pool_loader::PoolLoader;

/// Raydium Swap指令的账户布局
/// 0: Token Program
/// 1: AMM ID
/// 2: AMM Authority
/// 3: AMM Open Orders
/// 4: AMM Target Orders  
/// 5: Pool Coin Token Account
/// 6: Pool PC Token Account
/// 7: Serum Program ID
/// 8: Serum Market
/// 9: Serum Bids
/// 10: Serum Asks
/// 11: Serum Event Queue
/// 12: Serum Coin Vault Account
/// 13: Serum PC Vault Account 
/// 14: Serum Vault Signer
/// 15: User Source Token Account
/// 16: User Destination Token Account
/// 17: User Owner

pub fn parse_raydium_amm_v4_swap(
    signature: &str,
    account_keys: &[String],
    instruction_data: &[u8],
    pre_balances: &[u64],
    post_balances: &[u64],
    pre_token_balances: &[serde_json::Value],
    post_token_balances: &[serde_json::Value],
    _logs: &[String],
) -> Result<Option<TradeDetails>> {
    // 验证是否为swap指令
    if instruction_data.is_empty() || instruction_data[0] != RAYDIUM_AMM_SWAP_INSTRUCTION {
        return Ok(None);
    }
    
    // 解析指令数据获取交易金额
    let (amount_in, _min_amount_out) = parse_swap_instruction_data(instruction_data)?;
    
    // 获取账户信息
    let user_wallet = &account_keys[0];
    let _pool_amm = &account_keys[1];
    let _pool_coin_account = &account_keys[5];
    let _pool_pc_account = &account_keys[6];

    // 动态查找目标Token账户（属于user_wallet且mint不是WSOL）
    let mut user_dest_account: Option<&str> = None;
    for balance in pre_token_balances {
        let owner = balance.get("owner").and_then(|o| o.as_str()).unwrap_or("");
        let mint = balance.get("mint").and_then(|m| m.as_str()).unwrap_or("");
        if owner == user_wallet && mint != WSOL_MINT {
            if let Some(account_index) = balance.get("accountIndex").and_then(|i| i.as_u64()) {
                if (account_index as usize) < account_keys.len() {
                    user_dest_account = Some(&account_keys[account_index as usize]);
                    break;
                }
            }
        }
    }
    let user_dest_account = user_dest_account.ok_or_else(|| anyhow::anyhow!("未找到目标Token账户，account_keys不足或数据异常"))?;

    // 分析代币余额变化来确定交易方向和实际金额
    let (trade_direction, token_in_info, token_out_info, actual_amount_out) = 
        analyze_token_changes(
            pre_token_balances,
            post_token_balances,
            user_wallet,
            user_dest_account,
            amount_in,
        )?;
    
    // 计算价格
    let price = calculate_price(
        amount_in,
        actual_amount_out,
        &token_in_info,
        &token_out_info,
        &trade_direction,
    )?;
    
    // 计算gas费
    let gas_fee = calculate_gas_fee(pre_balances, post_balances, 0); // user_index
    
    // 获取池子地址（AMM ID）
    let pool_address = &account_keys[1];
    let loader = PoolLoader::load();
    let pool_param = loader.find_amm_by_pool(pool_address);
    let program_id = pool_param.and_then(|p| p.program_id.clone()).unwrap_or(crate::types::RAYDIUM_AMM_V4.to_string());
    let trade_details = TradeDetails {
        signature: signature.to_string(),
        wallet: Pubkey::from_str(user_wallet)?,
        dex_type: DexType::RaydiumAmmV4,
        trade_direction,
        token_in: token_in_info,
        token_out: token_out_info,
        amount_in,
        amount_out: actual_amount_out,
        price,
        pool_address: Pubkey::from_str(pool_address)? ,
        timestamp: Utc::now().timestamp(),
        gas_fee,
        program_id: Pubkey::from_str(&program_id)?,
    };
    
    info!("成功解析Raydium交易:");
    info!("  方向: {:?}", trade_details.trade_direction);
    info!("  输入: {} {}", 
        format_token_amount(amount_in, trade_details.token_in.decimals),
        trade_details.token_in.symbol.as_ref().unwrap_or(&"未知".to_string())
    );
    info!("  输出: {} {}",
        format_token_amount(actual_amount_out, trade_details.token_out.decimals),
        trade_details.token_out.symbol.as_ref().unwrap_or(&"未知".to_string())
    );
    info!("  价格: {:.6}", price);
    
    Ok(Some(trade_details))
}

/// 解析swap指令数据
fn parse_swap_instruction_data(data: &[u8]) -> Result<(u64, u64)> {
    if data.len() < 17 {
        return Err(anyhow::anyhow!("指令数据长度不足"));
    }
    
    // Raydium swap指令格式：
    // [0]: 指令类型 (9)
    // [1-8]: amount_in (u64, little-endian)
    // [9-16]: min_amount_out (u64, little-endian)
    
    let amount_in = u64::from_le_bytes(
        data[1..9].try_into()
            .context("无法解析amount_in")?
    );
    
    let min_amount_out = u64::from_le_bytes(
        data[9..17].try_into()
            .context("无法解析min_amount_out")?
    );
    
    Ok((amount_in, min_amount_out))
}

/// 分析代币余额变化
fn analyze_token_changes(
    pre_token_balances: &[serde_json::Value],
    post_token_balances: &[serde_json::Value],
    user_source_account: &str,
    user_dest_account: &str,
    _amount_in: u64,
) -> Result<(TradeDirection, TokenInfo, TokenInfo, u64)> {
    // 查找源账户和目标账户的mint
    let source_mint = find_mint_for_account(pre_token_balances, user_source_account)?;
    let dest_mint = find_mint_for_account(pre_token_balances, user_dest_account)?;
    
    // 计算实际的输出金额
    let (_, dest_post) = parser::TransactionParser::calculate_token_balance_change(
        pre_token_balances,
        post_token_balances,
        &dest_mint,
    )?;
    
    let (dest_pre, _) = parser::TransactionParser::calculate_token_balance_change(
        pre_token_balances,
        post_token_balances,
        &dest_mint,
    )?;
    
    let actual_amount_out = dest_post.saturating_sub(dest_pre);
    
    // 判断交易方向
    let (trade_direction, token_in_info, token_out_info) = if source_mint == WSOL_MINT {
        // SOL -> Token (买入)
        (
            TradeDirection::Buy,
            TokenInfo {
                mint: Pubkey::from_str(&source_mint)?,
                symbol: Some("SOL".to_string()),
                decimals: 9,
            },
            TokenInfo {
                mint: Pubkey::from_str(&dest_mint)?,
                symbol: get_token_symbol(&dest_mint),
                decimals: get_token_decimals(&dest_mint),
            },
        )
    } else {
        // Token -> SOL (卖出)
        (
            TradeDirection::Sell,
            TokenInfo {
                mint: Pubkey::from_str(&source_mint)?,
                symbol: get_token_symbol(&source_mint),
                decimals: get_token_decimals(&source_mint),
            },
            TokenInfo {
                mint: Pubkey::from_str(&dest_mint)?,
                symbol: Some("SOL".to_string()),
                decimals: 9,
            },
        )
    };
    
    Ok((trade_direction, token_in_info, token_out_info, actual_amount_out))
}

/// 查找账户对应的mint地址
fn find_mint_for_account(
    token_balances: &[serde_json::Value],
    _account: &str,
) -> Result<String> {
    for balance in token_balances {
        if let Some(_owner) = balance.get("accountIndex").and_then(|i| i.as_u64()) {
            // 这里需要匹配账户索引，实际实现中需要根据account_keys来找到正确的索引
            if let Some(mint) = balance.get("mint").and_then(|m| m.as_str()) {
                return Ok(mint.to_string());
            }
        }
    }
    
    // 如果找不到，可能是SOL账户
    Ok(WSOL_MINT.to_string())
}

/// 计算价格
fn calculate_price(
    amount_in: u64,
    amount_out: u64,
    token_in: &TokenInfo,
    token_out: &TokenInfo,
    direction: &TradeDirection,
) -> Result<f64> {
    let in_amount_decimal = amount_in as f64 / 10f64.powi(token_in.decimals as i32);
    let out_amount_decimal = amount_out as f64 / 10f64.powi(token_out.decimals as i32);
    
    match direction {
        TradeDirection::Buy => {
            // 买入时，价格 = SOL数量 / Token数量
            Ok(in_amount_decimal / out_amount_decimal)
        }
        TradeDirection::Sell => {
            // 卖出时，价格 = SOL数量 / Token数量
            Ok(out_amount_decimal / in_amount_decimal)
        }
    }
}

/// 计算gas费
fn calculate_gas_fee(pre_balances: &[u64], post_balances: &[u64], user_index: usize) -> u64 {
    if user_index < pre_balances.len() && user_index < post_balances.len() {
        pre_balances[user_index].saturating_sub(post_balances[user_index])
    } else {
        0
    }
}

/// 格式化代币数量
fn format_token_amount(amount: u64, decimals: u8) -> String {
    let divisor = 10f64.powi(decimals as i32);
    format!("{:.4}", amount as f64 / divisor)
}

/// 获取代币符号（这里可以接入代币信息服务）
fn get_token_symbol(mint: &str) -> Option<String> {
    match mint {
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" => Some("USDC".to_string()),
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB" => Some("USDT".to_string()),
        _ => None,
    }
}

/// 获取代币精度（实际应用中应该从链上获取）
fn get_token_decimals(mint: &str) -> u8 {
    match mint {
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" => 6, // USDC
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB" => 6, // USDT
        _ => 9, // 默认9位精度
    }
}