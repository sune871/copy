# Solana钱包监控和跟单程序

一个基于Rust的Solana钱包监控工具，支持实时监控目标钱包的DEX交易并执行跟单操作。

## 功能特性

- 🔍 **实时监控**: 通过gRPC实时监控目标钱包交易
- 📊 **交易解析**: 支持Raydium CPMM、Pump.fun等DEX交易解析
- 🤖 **自动跟单**: 可配置的自动跟单功能
- 📝 **交易记录**: 完整的交易记录和日志
- 🧪 **测试模式**: 内置测试和模拟功能

## 支持的DEX

- **Raydium CPMM**: 恒定乘积做市商
- **Raydium AMM V4**: 自动做市商V4
- **Pump.fun**: Pump.fun代币交易
- **Raydium CLMM**: 集中流动性做市商

## 快速开始

### 1. 编译项目

```bash
# 在本地编译（推荐）
cargo build --release

# 或在服务器上编译（需要足够内存）
cargo build -j 1
```

### 2. 配置程序

编辑 `config.json` 文件：

```json
{
  "rpc_url": "https://api.mainnet-beta.solana.com",
  "target_wallets": [
    "CuwxHwz42cNivJqWGBk6HcVvfGq47868Mo6zi4u6z9vC"
  ],
  "trade_execution": {
    "enabled": false,
    "copy_wallet_private_key": "your_private_key_here",
    "min_trade_amount": 0.1,
    "max_trade_amount": 10.0
  }
}
```

### 3. 运行程序

#### 测试模式（推荐先运行）
```bash
# 运行功能测试
cargo run --test

# 运行性能测试
cargo run --performance

# 运行模拟监控
cargo run --mock
```

#### 正常运行模式
```bash
# 启动监控程序
cargo run

# 或运行编译后的二进制文件
./target/release/wallet_copier
```

## 运行模式说明

### 🧪 测试模式 (`--test`)
- 验证配置加载和解析
- 测试交易数据结构
- 验证交易记录功能
- 模拟不同类型的交易处理
- **无需网络连接，安全测试**

### ⚡ 性能测试 (`--performance`)
- 模拟处理1000个交易
- 测量处理速度和内存使用
- 验证程序性能表现

### 🎭 模拟监控模式 (`--mock`)
- 生成模拟交易数据
- 测试完整的交易处理流程
- 验证交易记录和日志功能
- **适合在低配置服务器上测试**

### 🚀 正常运行模式
- 连接真实gRPC服务
- 监控真实钱包交易
- 执行实际的跟单操作

## 项目结构

```
src/
├── main.rs              # 主程序入口
├── config.rs            # 配置管理
├── types.rs             # 数据类型定义
├── grpc_monitor.rs      # gRPC监控模块
├── parser/              # 交易解析器
│   ├── mod.rs
│   ├── raydium.rs
│   └── raydium_cpmm.rs
├── dex/                 # DEX协议支持
│   ├── raydium/
│   └── pump/
├── trade_executor.rs    # 交易执行器
├── trade_recorder.rs    # 交易记录器
├── test_runner.rs       # 测试运行器
└── mock_monitor.rs      # 模拟监控器
```

## 配置说明

### 必需配置
- `rpc_url`: Solana RPC节点URL
- `target_wallets`: 要监控的钱包地址列表

### 可选配置
- `trade_execution.enabled`: 是否启用跟单功能
- `trade_execution.copy_wallet_private_key`: 跟单钱包私钥
- `trade_execution.min_trade_amount`: 最小跟单金额（SOL）
- `trade_execution.max_trade_amount`: 最大跟单金额（SOL）

## 安全注意事项

⚠️ **重要安全提醒**:
- 私钥文件请妥善保管，不要上传到公共仓库
- 建议在测试网络上先验证功能
- 跟单功能涉及真实资金，请谨慎使用
- 建议使用专门的跟单钱包，不要使用主钱包

## 故障排除

### 编译问题
- **内存不足**: 在低配置服务器上编译时可能遇到内存不足问题
  - 解决方案：在本地编译后上传二进制文件到服务器
  - 或升级服务器内存到至少4GB

### 运行问题
- **网络连接**: 确保服务器能访问Solana RPC节点
- **权限问题**: 确保程序有写入日志和交易记录的权限
- **配置错误**: 检查config.json格式是否正确

## 开发说明

### 添加新的DEX支持
1. 在 `src/dex/` 下创建新的DEX模块
2. 在 `src/parser/` 下添加对应的解析器
3. 在 `src/types.rs` 中添加新的DEX类型
4. 更新 `src/grpc_monitor.rs` 中的DEX识别逻辑

### 测试新功能
```bash
# 运行所有测试
cargo run --test

# 运行模拟监控测试新功能
cargo run --mock
```

## 许可证

本项目仅供学习和研究使用，请遵守相关法律法规。

## 贡献

欢迎提交Issue和Pull Request来改进这个项目！ 