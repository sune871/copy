# Solana 钱包监控和跟单程序

这是一个用于监控 Solana 钱包交易并自动跟单的程序。

## 功能特性

- ✅ 实时监控目标钱包的交易活动
- ✅ 解析 Raydium CPMM 和 Pump.fun 交易
- ✅ 自动跟单买入/卖出操作
- ✅ 交易记录保存和分析
- ✅ 可配置的交易参数（金额限制、滑点等）

## 配置说明

### config.json 配置

```json
{
    "rpc_url": "你的RPC节点地址",
    "target_wallets": ["目标钱包地址"],
    "copy_wallet_private_key": "跟单钱包私钥",
    "trading_settings": {
        "max_position_size": 0.1,
        "slippage_tolerance": 0.05,
        "gas_price_multiplier": 1.2
    },
    "execution_config": {
        "enabled": true,
        "min_trade_amount": 0.01,
        "max_trade_amount": 0.5,
        "max_position_size": 0.1,
        "slippage_tolerance": 0.05,
        "gas_price_multiplier": 1.2
    }
}
```

### 配置参数说明

- `rpc_url`: Solana RPC 节点地址
- `target_wallets`: 要监控的目标钱包地址列表
- `copy_wallet_private_key`: 跟单钱包的私钥（base58格式）
- `execution_config.enabled`: 是否启用跟单功能
- `execution_config.min_trade_amount`: 最小跟单金额（SOL）
- `execution_config.max_trade_amount`: 最大跟单金额（SOL）
- `execution_config.slippage_tolerance`: 滑点容忍度
- `execution_config.gas_price_multiplier`: Gas价格倍数

## 使用方法

### 1. 在 Ubuntu 系统上编译

```bash
# 进入项目目录
cd /path/to/your/project

# 编译项目
cargo build --release

# 运行程序
cargo run --release
```

### 2. 检查配置

确保 `config.json` 中的配置正确：
- RPC 节点地址可访问
- 目标钱包地址正确
- 跟单钱包私钥正确且有足够余额

### 3. 监控输出

程序运行后会显示：
- 目标钱包的交易活动
- 解析的交易详情
- 跟单执行结果
- 交易记录保存状态

## 安全注意事项

⚠️ **重要提醒**：
- 私钥文件要妥善保管，不要泄露
- 建议使用专门的跟单钱包，不要使用主钱包
- 设置合理的交易金额限制
- 定期检查跟单钱包余额

## 交易记录

程序会自动保存交易记录到 `trades/trade_records.json` 文件，包含：
- 原始交易签名
- 跟单交易签名
- 交易方向（买入/卖出）
- 交易金额和价格
- 执行结果和错误信息

## 故障排除

### 常见问题

1. **编译错误**：确保在 Ubuntu 环境下编译
2. **连接失败**：检查 RPC 节点地址和网络连接
3. **私钥错误**：确保私钥格式正确（base58）
4. **余额不足**：检查跟单钱包余额

### 日志级别

可以通过修改日志级别来获取更详细的信息：
```rust
tracing_subscriber::fmt()
    .with_max_level(tracing::Level::DEBUG)  // 改为 DEBUG 获取更多信息
    .init();
```

## 免责声明

本程序仅供学习和研究使用，使用者需要：
- 自行承担交易风险
- 确保遵守当地法律法规
- 谨慎使用自动交易功能
- 定期备份重要数据

## 技术架构

- **监控模块**: gRPC 实时监控
- **解析模块**: 交易数据解析
- **执行模块**: 交易执行和跟单
- **记录模块**: 交易记录保存
- **配置模块**: 参数配置管理 