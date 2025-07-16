use anyhow::{Result, Context};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use tracing::{info, debug, warn};
use crate::types::{TradeDetails, TradeDirection, TokenInfo, DexType};
use crate::types::{RAYDIUM_CPMM_SWAP_BASE_INPUT, RAYDIUM_CPMM_SWAP_BASE_OUTPUT};
use chrono::Utc;
use crate::types::WSOL_MINT;
use wallet_copier::pool_loader::PoolLoader;

/// 移除get_sol_usd_price和parse_raydium_cpmm_swap_with_usd相关内容

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
    let pool_address = &account_keys[pool_account_index];
    let loader = PoolLoader::load();
    let pool_param = loader.find_cpmm_by_pool(pool_address);
    let program_id = pool_param.and_then(|p| p.program_id.clone()).unwrap_or(crate::types::RAYDIUM_CPMM.to_string());
    
    // 查找用户钱包
    let user_wallet = find_user_wallet(account_keys)?;

    // 1. 找到目标钱包的WSOL账户余额变化
    let mut wsol_pre = 0u64;
    let mut wsol_post = 0u64;
    let mut found_wsol = false;
    for (pre, post) in pre_token_balances.iter().zip(post_token_balances.iter()) {
        let mint = pre.get("mint").and_then(|m| m.as_str()).unwrap_or("");
        let owner = pre.get("owner").and_then(|o| o.as_str()).unwrap_or("");
        if mint == WSOL_MINT && owner == user_wallet.to_string() {
            wsol_pre = pre.get("uiTokenAmount").and_then(|ui| ui.get("amount")).and_then(|a| a.as_str()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            wsol_post = post.get("uiTokenAmount").and_then(|ui| ui.get("amount")).and_then(|a| a.as_str()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            found_wsol = true;
            break;
        }
    }
    // 如果没找到WSOL账户，尝试用主账户SOL余额变化
    let mut sol_change = 0u64;
    if !found_wsol {
        let user_index = account_keys.iter().position(|k| k == &user_wallet.to_string());
        if let Some(idx) = user_index {
            if idx < pre_balances.len() && idx < post_balances.len() {
                let pre_sol = pre_balances[idx];
                let post_sol = post_balances[idx];
                sol_change = if pre_sol > post_sol { pre_sol - post_sol } else { post_sol - pre_sol };
                tracing::warn!("未找到WSOL账户，使用主账户SOL余额变化: {}", sol_change);
            }
        }
    }
    let wsol_change = if found_wsol {
        if wsol_pre > wsol_post { wsol_pre - wsol_post } else { wsol_post - wsol_pre }
    } else {
        sol_change
    };
    // 判断买入还是卖出
    let trade_direction = if wsol_pre > wsol_post {
        TradeDirection::Buy
    } else {
        TradeDirection::Sell
    };

    // 2. 解析目标代币的余额变化
    let wsol_mint = WSOL_MINT;
    let mut max_in_token = None;
    let mut max_out_token = None;
    let mut max_in_amount = 0u64;
    let mut max_out_amount = 0u64;
    for (pre, post) in pre_token_balances.iter().zip(post_token_balances.iter()) {
        let mint = pre.get("mint").and_then(|m| m.as_str()).unwrap_or("");
        let owner = pre.get("owner").and_then(|o| o.as_str()).unwrap_or("");
        if owner == user_wallet.to_string() {
            let pre_amt = pre.get("uiTokenAmount").and_then(|ui| ui.get("amount")).and_then(|a| a.as_str()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            let post_amt = post.get("uiTokenAmount").and_then(|ui| ui.get("amount")).and_then(|a| a.as_str()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            if pre_amt > post_amt {
                let diff = pre_amt - post_amt;
                if diff > max_in_amount {
                    max_in_amount = diff;
                    max_in_token = Some(mint.to_string());
                }
            } else if post_amt > pre_amt {
                let diff = post_amt - pre_amt;
                if diff > max_out_amount {
                    max_out_amount = diff;
                    max_out_token = Some(mint.to_string());
                }
            }
        }
    }
    // 判断方向
    let trade_direction = if let Some(ref in_token) = max_in_token {
        if in_token == wsol_mint {
            TradeDirection::Buy
        } else {
            TradeDirection::Sell
        }
    } else {
        TradeDirection::Sell
    };
    // token_in/token_out/amount_in/amount_out始终按最大减少/最大增加token
    let token_in_mint = max_in_token.clone().unwrap_or_else(|| "So11111111111111111111111111111111111111112".to_string());
    let token_out_mint = max_out_token.clone().unwrap_or_else(|| "So11111111111111111111111111111111111111112".to_string());
    let amount_in = max_in_amount;
    let amount_out = max_out_amount;

    // 3. 构造TradeDetails
    // 计算价格
    let price = calculate_price(
        amount_in,
        amount_out,
        &TokenInfo {
            mint: Pubkey::from_str(&token_in_mint)?,
            symbol: get_token_symbol(&token_in_mint),
            decimals: get_token_decimals(&token_in_mint),
        },
        &TokenInfo {
            mint: Pubkey::from_str(&token_out_mint)?,
            symbol: get_token_symbol(&token_out_mint),
            decimals: get_token_decimals(&token_out_mint),
        },
        &trade_direction,
    )?;
    // 计算gas费
    let gas_fee = calculate_gas_fee(pre_balances, post_balances, account_keys)?;
    let trade_details = TradeDetails {
        signature: signature.to_string(),
        wallet: user_wallet,
        dex_type: DexType::RaydiumCPMM,
        trade_direction,
        token_in: TokenInfo {
            mint: Pubkey::from_str(&token_in_mint)?,
            symbol: get_token_symbol(&token_in_mint),
            decimals: get_token_decimals(&token_in_mint),
        },
        token_out: TokenInfo {
            mint: Pubkey::from_str(&token_out_mint)?,
            symbol: get_token_symbol(&token_out_mint),
            decimals: get_token_decimals(&token_out_mint),
        },
        amount_in,
        amount_out,
        price,
        pool_address: Pubkey::from_str(pool_address)? ,
        timestamp: Utc::now().timestamp(),
        gas_fee,
        program_id: Pubkey::from_str(&program_id)?,
    };
    
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
    let input_mint = None;
    let input_amount = None;
    let output_mint = None;
    let output_amount = None;
    
    for log in logs {
        // 查找包含swap信息的日志行
        if log.contains("base_input") || log.contains("base_output") {
            debug!("Swap日志: {}", log);
        }
        
        // 尝试从日志中提取金额信息
        if log.contains("amount_in:") || log.contains("input_amount:") {
            if let Some(_amount) = extract_number_from_log(log, "amount_in:") {
                // input_amount = Some(amount);
            }
        }
        
        if log.contains("amount_out:") || log.contains("output_amount:") {
            if let Some(_amount) = extract_number_from_log(log, "amount_out:") {
                // output_amount = Some(amount);
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
            if pre_amount != post_amount && mint != "So11111111111111111111111111111111111111112" {
                token_changes.push((mint.to_string(), pre_amount, post_amount));
            }
        }
    }
    
    // 分析SOL余额变化 - 降低阈值，提高检测灵敏度
    let sol_change = if user_index < pre_balances.len() && user_index < post_balances.len() {
        let pre_sol = pre_balances[user_index];
        let post_sol = post_balances[user_index];
        let sol_diff = if pre_sol > post_sol {
            pre_sol - post_sol
        } else {
            post_sol - pre_sol
        };
        
        // 降低阈值到 1 SOL (1_000_000_000 lamports)
        if sol_diff > 1_000_000_000 {
            if pre_sol > post_sol {
                // SOL减少，可能是买入
                Some((sol_diff, TradeDirection::Buy))
            } else {
                // SOL增加，可能是卖出
                Some((sol_diff, TradeDirection::Sell))
            }
        } else {
            // 即使SOL变化很小，也尝试检测
            debug!("SOL变化较小: {} lamports", sol_diff);
            if pre_sol > post_sol {
                Some((sol_diff, TradeDirection::Buy))
            } else {
                Some((sol_diff, TradeDirection::Sell))
            }
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
                            mint: Pubkey::from_str("So11111111111111111111111111111111111111112")?,
                            symbol: Some("SOL".to_string()),
                            decimals: 9,
                        },
                        TokenInfo {
                            mint: Pubkey::from_str(&token_mint)?,
                            symbol: get_token_symbol(&token_mint),
                            decimals: get_token_decimals(&token_mint),
                        },
                        sol_amount,
                        token_amount,
                    ))
                }
                TradeDirection::Sell => {
                    Ok((
                        TradeDirection::Sell,
                        TokenInfo {
                            mint: Pubkey::from_str(&token_mint)?,
                            symbol: get_token_symbol(&token_mint),
                            decimals: get_token_decimals(&token_mint),
                        },
                        TokenInfo {
                            mint: Pubkey::from_str("So11111111111111111111111111111111111111112")?,
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
                // 暂时返回一个默认的交易信息
                Ok((
                    direction,
                    TokenInfo {
                        mint: Pubkey::from_str("So11111111111111111111111111111111111111112")?,
                        symbol: Some("SOL".to_string()),
                        decimals: 9,
                    },
                    TokenInfo {
                        mint: Pubkey::from_str("So11111111111111111111111111111111111111112")?,
                        symbol: Some("未知".to_string()),
                        decimals: 9,
                    },
                    sol_amount,
                    swap_info.output_amount,
                ))
            } else {
                Err(anyhow::anyhow!("未找到代币余额变化"))
            }
        }
    } else {
        // 如果找不到SOL变化，尝试从代币变化推断
        if let Some((token_mint, pre_token, post_token)) = token_changes.first() {
            let token_amount = if *post_token > *pre_token {
                *post_token - *pre_token
            } else {
                *pre_token - *post_token
            };
            
            // 假设这是买入操作
            warn!("未检测到SOL变化，假设为买入操作");
            Ok((
                TradeDirection::Buy,
                TokenInfo {
                    mint: Pubkey::from_str("So11111111111111111111111111111111111111112")?,
                    symbol: Some("SOL".to_string()),
                    decimals: 9,
                },
                TokenInfo {
                    mint: Pubkey::from_str(&token_mint)?,
                    symbol: get_token_symbol(&token_mint),
                    decimals: get_token_decimals(&token_mint),
                },
                0, // SOL数量未知
                token_amount,
            ))
        } else {
            Err(anyhow::anyhow!("未找到明显的SOL余额变化"))
        }
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
        "So11111111111111111111111111111111111111112" => Some("SOL".to_string()),
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" => Some("USDC".to_string()),
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB" => Some("USDT".to_string()),
        _ => None,
    }
}

/// 获取代币精度
fn get_token_decimals(mint: &str) -> u8 {
    match mint {
        "So11111111111111111111111111111111111111112" => 9, // SOL/WSOL
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" => 6, // USDC
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB" => 6, // USDT
        // 你可以在这里补充更多常见币
        _ => 6, // 默认6位精度，适配大部分新币
    }
}