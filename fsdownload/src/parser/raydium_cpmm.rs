use anyhow::{Result, Context};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use tracing::{info, debug, warn};
use crate::types::{TradeDetails, DexType, TradeDirection, TokenInfo, WSOL_MINT, RAYDIUM_CPMM_SWAP_BASE_INPUT, RAYDIUM_CPMM_SWAP_BASE_OUTPUT};
use crate::parser;
use crate::dex::raydium::RaydiumCpAmmInfo;
use chrono::Utc;

pub fn parse_raydium_cpmm_swap(
    signature: &str,
    account_keys: &[String],
    instruction_data: &[u8],
    pre_balances: &[u64],
    post_balances: &[u64],
    pre_token_balances: &[serde_json::Value],
    post_token_balances: &[serde_json::Value],
    logs: &[String],
) -> Result<Option<TradeDetails>> {
    // 检查指令数据长度
    if instruction_data.len() < 8 {
        return Ok(None);
    }
    
    // 检查是否为CPMM swap指令
    let discriminator = &instruction_data[0..8];
    let is_swap_base_input = discriminator == RAYDIUM_CPMM_SWAP_BASE_INPUT;
    let is_swap_base_output = discriminator == RAYDIUM_CPMM_SWAP_BASE_OUTPUT;
    
    if !is_swap_base_input && !is_swap_base_output {
        debug!("不是CPMM swap指令");
        return Ok(None);
    }
    
    let swap_type = if is_swap_base_input { "swap_base_input" } else { "swap_base_output" };
    info!("检测到Raydium CPMM {} 指令", swap_type);
    
    // 解析日志获取交易详情
    let swap_info = parse_swap_info_from_logs(logs)?;
    
    // 查找池子账户并获取池子信息
    let pool_account_index = find_pool_account_index(account_keys)?;
    let pool_address = Pubkey::from_str(&account_keys[pool_account_index])?;
    
    // 查找用户钱包
    let user_wallet = find_user_wallet(account_keys)?;
    
    // 分析代币余额变化
    let (trade_direction, token_in_info, token_out_info, actual_amount_in, actual_amount_out) = 
        analyze_token_changes_from_logs_and_balances(
            &swap_info,
            pre_token_balances,
            post_token_balances,
            pre_balances,
            post_balances,
            account_keys,
            &user_wallet,
        )?;
    
    // 计算价格
    let price = calculate_price(
        actual_amount_in,
        actual_amount_out,
        &token_in_info,
        &token_out_info,
        &trade_direction,
    )?;
    
    // 计算gas费
    let gas_fee = calculate_gas_fee(pre_balances, post_balances, account_keys)?;
    
    let trade_details = TradeDetails {
        signature: signature.to_string(),
        wallet: user_wallet,
        dex_type: DexType::RaydiumCPMM,
        trade_direction,
        token_in: token_in_info,
        token_out: token_out_info,
        amount_in: actual_amount_in,
        amount_out: actual_amount_out,
        price,
        pool_address,
        timestamp: Utc::now().timestamp(),
        gas_fee,
        program_id: Pubkey::from_str(crate::types::RAYDIUM_CPMM)?,
    };
    
    info!("成功解析Raydium CPMM交易:");
    info!("  交易类型: {}", swap_type);
    info!("  方向: {:?}", trade_details.trade_direction);
    info!("  输入: {} {}", 
        format_token_amount(actual_amount_in, trade_details.token_in.decimals),
        trade_details.token_in.symbol.as_ref().unwrap_or(&"未知".to_string())
    );
    info!("  输出: {} {}",
        format_token_amount(actual_amount_out, trade_details.token_out.decimals),
        trade_details.token_out.symbol.as_ref().unwrap_or(&"未知".to_string())
    );
    info!("  价格: {:.8} SOL/代币", price);
    info!("  Gas费: {:.6} SOL", gas_fee as f64 / 1e9);
    
    Ok(Some(trade_details))
}

/// 从日志中解析交易信息
#[derive(Debug)]
struct SwapInfo {
    input_mint: String,
    input_amount: u64,
    output_mint: String,
    output_amount: u64,
}

fn parse_swap_info_from_logs(logs: &[String]) -> Result<SwapInfo> {
    // 查找包含swap信息的日志
    // Raydium CPMM的日志格式通常包含输入输出信息
    let mut input_mint = None;
    let mut input_amount = None;
    let mut output_mint = None;
    let mut output_amount = None;
    
    for log in logs {
        // 查找包含swap信息的日志行
        if log.contains("base_input") || log.contains("base_output") {
            debug!("Swap日志: {}", log);
        }
        
        // 尝试从日志中提取金额信息
        if log.contains("amount_in:") || log.contains("input_amount:") {
            if let Some(amount) = extract_number_from_log(log, "amount_in:") {
                input_amount = Some(amount);
            }
        }
        
        if log.contains("amount_out:") || log.contains("output_amount:") {
            if let Some(amount) = extract_number_from_log(log, "amount_out:") {
                output_amount = Some(amount);
            }
        }
    }
    
    // 如果无法从日志中获取完整信息，返回默认值
    Ok(SwapInfo {
        input_mint: input_mint.unwrap_or_default(),
        input_amount: input_amount.unwrap_or(0),
        output_mint: output_mint.unwrap_or_default(),
        output_amount: output_amount.unwrap_or(0),
    })
}

/// 从日志中提取数字
fn extract_number_from_log(log: &str, key: &str) -> Option<u64> {
    if let Some(pos) = log.find(key) {
        let start = pos + key.len();
        let remaining = &log[start..];
        let number_str: String = remaining
            .chars()
            .skip_while(|c| c.is_whitespace() || *c == ':')
            .take_while(|c| c.is_numeric())
            .collect();
        
        number_str.parse::<u64>().ok()
    } else {
        None
    }
}

/// 查找池子账户索引
fn find_pool_account_index(account_keys: &[String]) -> Result<usize> {
    // 池子账户通常在前几个位置
    // CPMM池子账户的特征：不是系统程序，不是代币程序，不是CPMM程序本身
    for (i, account) in account_keys.iter().enumerate() {
        // 跳过已知的程序账户
        if account == crate::types::RAYDIUM_CPMM ||
           account == "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA" ||
           account == "11111111111111111111111111111111" ||
           account.contains("oracle") ||
           account.contains("authority") {
            continue;
        }
        
        // 池子账户通常在索引1-5之间
        if i >= 1 && i <= 5 {
            debug!("可能的池子账户在索引 {}: {}", i, account);
            return Ok(i);
        }
    }
    
    // 默认返回索引1
    Ok(1)
}

/// 查找用户钱包地址
fn find_user_wallet(account_keys: &[String]) -> Result<Pubkey> {
    // 目标钱包地址
    const TARGET_WALLET: &str = "CuwxHwz42cNivJqWGBk6HcVvfGq47868Mo6zi4u6z9vC";
    
    for account in account_keys {
        if account == TARGET_WALLET {
            return Pubkey::from_str(account).context("无法解析用户钱包地址");
        }
    }
    
    // 如果没找到目标钱包，查找第一个非程序账户
    for account in account_keys {
        if !account.contains("Program") && 
           !account.contains("oracle") &&
           !account.contains("authority") &&
           account != "11111111111111111111111111111111" {
            return Pubkey::from_str(account).context("无法解析用户钱包地址");
        }
    }
    
    Err(anyhow::anyhow!("未找到用户钱包"))
}

/// 分析代币余额变化
fn analyze_token_changes_from_logs_and_balances(
    swap_info: &SwapInfo,
    pre_token_balances: &[serde_json::Value],
    post_token_balances: &[serde_json::Value],
    pre_balances: &[u64],
    post_balances: &[u64],
    account_keys: &[String],
    user_wallet: &Pubkey,
) -> Result<(TradeDirection, TokenInfo, TokenInfo, u64, u64)> {
    // 查找用户账户索引
    let user_index = account_keys.iter()
        .position(|k| k == &user_wallet.to_string())
        .context("未找到用户账户索引")?;
    
    // 收集所有代币余额变化
    let mut token_changes = Vec::new();
    
    // 分析前后代币余额
    for (pre_balance, post_balance) in pre_token_balances.iter().zip(post_token_balances.iter()) {
        if let (Some(mint), Some(pre_amount), Some(post_amount)) = (
            pre_balance.get("mint").and_then(|m| m.as_str()),
            extract_token_amount(pre_balance).ok(),
            extract_token_amount(post_balance).ok(),
        ) {
            if pre_amount != post_amount && mint != WSOL_MINT {
                token_changes.push((mint.to_string(), pre_amount, post_amount));
            }
        }
    }
    
    // 分析SOL余额变化
    let sol_change = if user_index < pre_balances.len() && user_index < post_balances.len() {
        let pre_sol = pre_balances[user_index];
        let post_sol = post_balances[user_index];
        
        if pre_sol > post_sol && (pre_sol - post_sol) > 10_000_000 {
            // SOL减少，可能是买入
            Some((pre_sol - post_sol, TradeDirection::Buy))
        } else if post_sol > pre_sol && (post_sol - pre_sol) > 10_000_000 {
            // SOL增加，可能是卖出
            Some((post_sol - pre_sol, TradeDirection::Sell))
        } else {
            None
        }
    } else {
        None
    };
    
    // 根据余额变化确定交易方向和金额
    if let Some((sol_amount, direction)) = sol_change {
        if let Some((token_mint, pre_token, post_token)) = token_changes.first() {
            let token_amount = if *post_token > *pre_token {
                *post_token - *pre_token
            } else {
                *pre_token - *post_token
            };
            
            match direction {
                TradeDirection::Buy => {
                    Ok((
                        TradeDirection::Buy,
                        TokenInfo {
                            mint: Pubkey::from_str(WSOL_MINT)?,
                            symbol: Some("SOL".to_string()),
                            decimals: 9,
                        },
                        TokenInfo {
                            mint: Pubkey::from_str(token_mint)?,
                            symbol: get_token_symbol(token_mint),
                            decimals: get_token_decimals(token_mint),
                        },
                        sol_amount,
                        token_amount,
                    ))
                }
                TradeDirection::Sell => {
                    Ok((
                        TradeDirection::Sell,
                        TokenInfo {
                            mint: Pubkey::from_str(token_mint)?,
                            symbol: get_token_symbol(token_mint),
                            decimals: get_token_decimals(token_mint),
                        },
                        TokenInfo {
                            mint: Pubkey::from_str(WSOL_MINT)?,
                            symbol: Some("SOL".to_string()),
                            decimals: 9,
                        },
                        token_amount,
                        sol_amount,
                    ))
                }
            }
        } else {
            // 如果没有代币变化，尝试使用日志中的信息
            if swap_info.input_amount > 0 && swap_info.output_amount > 0 {
                warn!("使用日志中的交易信息");
                // 这里需要更多逻辑来确定代币类型
            }
            Err(anyhow::anyhow!("未找到代币余额变化"))
        }
    } else {
        Err(anyhow::anyhow!("未找到明显的SOL余额变化"))
    }
}

/// 提取代币数量
fn extract_token_amount(balance: &serde_json::Value) -> Result<u64> {
    balance
        .get("uiTokenAmount")
        .and_then(|ui| ui.get("amount"))
        .and_then(|a| a.as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| anyhow::anyhow!("无法提取代币数量"))
}

/// 计算总gas费（包括网络费和0slot小费）
fn calculate_gas_fee(
    pre_balances: &[u64],
    post_balances: &[u64],
    account_keys: &[String],
) -> Result<u64> {
    let mut total_fee = 5000u64; // 基础网络费
    
    // 查找0slot账户
    for (i, account) in account_keys.iter().enumerate() {
        if account.contains("0slot") || account.contains("tip") {
            if i < pre_balances.len() && i < post_balances.len() {
                let tip = post_balances[i].saturating_sub(pre_balances[i]);
                if tip > 0 {
                    info!("检测到0slot小费: {} lamports ({:.6} SOL)", tip, tip as f64 / 1e9);
                    total_fee += tip;
                }
            }
        }
    }
    
    Ok(total_fee)
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
    
    if out_amount_decimal == 0.0 {
        return Err(anyhow::anyhow!("输出数量为0，无法计算价格"));
    }
    
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

/// 格式化代币数量
fn format_token_amount(amount: u64, decimals: u8) -> String {
    let divisor = 10f64.powi(decimals as i32);
    let value = amount as f64 / divisor;
    
    if value < 0.000001 {
        format!("{:.9}", value)
    } else if value < 1.0 {
        format!("{:.6}", value)
    } else if value < 1000.0 {
        format!("{:.4}", value)
    } else {
        format!("{:.2}", value)
    }
}

/// 获取代币符号
fn get_token_symbol(mint: &str) -> Option<String> {
    match mint {
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" => Some("USDC".to_string()),
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB" => Some("USDT".to_string()),
        _ => None,
    }
}

/// 获取代币精度
fn get_token_decimals(mint: &str) -> u8 {
    match mint {
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" => 6, // USDC
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB" => 6, // USDT
        _ => 9, // 默认9位精度
    }
}