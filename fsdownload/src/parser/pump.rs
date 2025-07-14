use anyhow::{Result, Context};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use tracing::{info, debug};
use crate::types::{TradeDetails, DexType, TradeDirection, TokenInfo, WSOL_MINT, PUMP_BUY_INSTRUCTION, PUMP_SELL_INSTRUCTION};
use crate::parser;
use chrono::Utc;

/// Pump.fun Buy指令的账户布局
/// 0: Pump Program
/// 1: Fee Recipient
/// 2: Mint (代币地址)
/// 3: Bonding Curve (联合曲线)
/// 4: Associated Bonding Curve
/// 5: User Token Account
/// 6: User (签名者)
/// 7: System Program
/// 8: Token Program
/// 9: Rent
/// 10: Event Authority
/// 11: Program

pub fn parse_pump_trade(
    signature: &str,
    account_keys: &[String],
    instruction_data: &[u8],
    pre_balances: &[u64],
    post_balances: &[u64],
    pre_token_balances: &[serde_json::Value],
    post_token_balances: &[serde_json::Value],
    logs: &[String],
) -> Result<Option<TradeDetails>> {
    if instruction_data.is_empty() {
        return Ok(None);
    }
    
    // 判断交易类型
    let instruction_type = instruction_data[0];
    let trade_direction = match instruction_type {
        PUMP_BUY_INSTRUCTION => TradeDirection::Buy,
        PUMP_SELL_INSTRUCTION => TradeDirection::Sell,
        _ => return Ok(None),
    };
    
    debug!("检测到Pump.fun {:?} 交易", trade_direction);
    
    // 解析指令数据
    let (_amount, _max_sol_cost) = parse_pump_instruction_data(instruction_data)?;
    
    // 获取关键账户
    let mint_address = &account_keys[2];
    let bonding_curve = &account_keys[3];
    let user_address = &account_keys[6];
    
    let user_wallet = Pubkey::from_str(user_address)
        .context("无法解析用户钱包地址")?;
    
    // 分析实际的交易金额和价格
    let (actual_sol_amount, actual_token_amount) = analyze_pump_trade(
        &trade_direction,
        pre_balances,
        post_balances,
        pre_token_balances,
        post_token_balances,
        account_keys,
        mint_address,
    )?;
    
    // 创建代币信息
    let (token_in, token_out, amount_in, amount_out) = match trade_direction {
        TradeDirection::Buy => {
            // 买入：SOL -> Token
            (
                TokenInfo {
                    mint: Pubkey::from_str(WSOL_MINT)?,
                    symbol: Some("SOL".to_string()),
                    decimals: 9,
                },
                TokenInfo {
                    mint: Pubkey::from_str(mint_address)?,
                    symbol: extract_token_symbol_from_logs(logs, mint_address),
                    decimals: get_pump_token_decimals(), // Pump代币通常是6位精度
                },
                actual_sol_amount,
                actual_token_amount,
            )
        }
        TradeDirection::Sell => {
            // 卖出：Token -> SOL
            (
                TokenInfo {
                    mint: Pubkey::from_str(mint_address)?,
                    symbol: extract_token_symbol_from_logs(logs, mint_address),
                    decimals: get_pump_token_decimals(),
                },
                TokenInfo {
                    mint: Pubkey::from_str(WSOL_MINT)?,
                    symbol: Some("SOL".to_string()),
                    decimals: 9,
                },
                actual_token_amount,
                actual_sol_amount,
            )
        }
    };
    
    // 计算价格（每个代币的SOL价格）
    let price = calculate_pump_price(actual_sol_amount, actual_token_amount, &trade_direction)?;
    
    // 计算gas费
    let user_index = account_keys.iter().position(|k| k == user_address).unwrap_or(0);
    let gas_fee = calculate_gas_fee(pre_balances, post_balances, user_index);
    
    let trade_details = TradeDetails {
        signature: signature.to_string(),
        wallet: user_wallet,
        dex_type: DexType::PumpFun,
        trade_direction,
        token_in,
        token_out,
        amount_in,
        amount_out,
        price,
        pool_address: Pubkey::from_str(bonding_curve)?,
        timestamp: Utc::now().timestamp(),
        gas_fee,
        program_id: Pubkey::from_str(crate::types::PUMP_FUN_PROGRAM)?,
    };
    
    info!("成功解析Pump.fun交易:");
    info!("  方向: {:?}", trade_details.trade_direction);
    info!("  输入: {} {}",
        format_amount(amount_in, trade_details.token_in.decimals),
        trade_details.token_in.symbol.as_ref().unwrap_or(&"未知".to_string())
    );
    info!("  输出: {} {}",
        format_amount(amount_out, trade_details.token_out.decimals),
        trade_details.token_out.symbol.as_ref().unwrap_or(&"未知".to_string())
    );
    info!("  价格: {:.8} SOL", price);
    info!("  Gas费: {:.6} SOL", gas_fee as f64 / 1e9);
    
    Ok(Some(trade_details))
}

/// 解析Pump指令数据
fn parse_pump_instruction_data(data: &[u8]) -> Result<(u64, u64)> {
    if data.len() < 17 {
        return Err(anyhow::anyhow!("Pump指令数据长度不足"));
    }
    
    // Pump指令格式：
    // [0]: 指令类型
    // [1-8]: amount (代币数量或SOL数量)
    // [9-16]: max_sol_cost (最大SOL成本，用于滑点保护)
    
    let amount = u64::from_le_bytes(
        data[1..9].try_into()
            .context("无法解析amount")?
    );
    
    let max_sol_cost = u64::from_le_bytes(
        data[9..17].try_into()
            .context("无法解析max_sol_cost")?
    );
    
    Ok((amount, max_sol_cost))
}

/// 分析Pump交易的实际金额
fn analyze_pump_trade(
    trade_direction: &TradeDirection,
    pre_balances: &[u64],
    post_balances: &[u64],
    pre_token_balances: &[serde_json::Value],
    post_token_balances: &[serde_json::Value],
    _account_keys: &[String],
    mint_address: &str,
) -> Result<(u64, u64)> {
    // 获取用户账户索引
    let user_index = 6; // Pump.fun中用户账户通常在索引6
    
    // 计算SOL变化
    let sol_change = if user_index < pre_balances.len() && user_index < post_balances.len() {
        let pre_sol = pre_balances[user_index];
        let post_sol = post_balances[user_index];
        pre_sol.abs_diff(post_sol)
    } else {
        0
    };
    
    // 计算代币变化
    let (pre_token, post_token) = parser::TransactionParser::calculate_token_balance_change(
        pre_token_balances,
        post_token_balances,
        mint_address,
    )?;
    
    let token_change = pre_token.abs_diff(post_token);
    
    // 根据交易方向返回正确的金额
    match trade_direction {
        TradeDirection::Buy => Ok((sol_change, token_change)),
        TradeDirection::Sell => Ok((sol_change, token_change)),
    }
}

/// 计算Pump价格
fn calculate_pump_price(sol_amount: u64, token_amount: u64, _direction: &TradeDirection) -> Result<f64> {
    let sol_decimal = sol_amount as f64 / 1e9;
    let token_decimal = token_amount as f64 / 1e6; // Pump代币通常是6位精度
    
    if token_decimal == 0.0 {
        return Err(anyhow::anyhow!("代币数量为0，无法计算价格"));
    }
    
    // 价格始终表示为每个代币的SOL价格
    Ok(sol_decimal / token_decimal)
}

/// 从日志中提取代币符号
fn extract_token_symbol_from_logs(logs: &[String], _mint: &str) -> Option<String> {
    // Pump.fun的日志中可能包含代币符号信息
    for log in logs {
        if log.contains("symbol:") || log.contains("Symbol:") {
            // 尝试从日志中提取符号
            if let Some(symbol_start) = log.find("symbol:") {
                let symbol_part = &log[symbol_start + 7..];
                if let Some(symbol_end) = symbol_part.find(' ') {
                    return Some(symbol_part[..symbol_end].trim().to_string());
                }
            }
        }
    }
    
    // 如果无法从日志中获取，返回None
    None
}

/// 获取Pump代币精度（通常为6）
fn get_pump_token_decimals() -> u8 {
    6
}

/// 计算gas费
fn calculate_gas_fee(pre_balances: &[u64], post_balances: &[u64], user_index: usize) -> u64 {
    // 计算用户SOL余额的额外减少（除了交易金额外的部分就是gas费）
    if user_index < pre_balances.len() && user_index < post_balances.len() {
        // 这需要更复杂的逻辑来区分交易金额和gas费
        // 简化处理：通常gas费在0.000005到0.00001 SOL之间
        5000 // 0.000005 SOL
    } else {
        0
    }
}

/// 格式化金额显示
fn format_amount(amount: u64, decimals: u8) -> String {
    let divisor = 10f64.powi(decimals as i32);
    let value = amount as f64 / divisor;
    
    if value < 0.0001 {
        format!("{:.8}", value)
    } else if value < 1.0 {
        format!("{:.6}", value)
    } else {
        format!("{:.4}", value)
    }
}