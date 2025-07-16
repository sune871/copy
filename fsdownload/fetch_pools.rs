use std::fs::File;
use std::io::Write;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 拉取Raydium AMM池子
    let amm_resp = reqwest::get("https://api.raydium.io/v2/main/pairs").await?;
    let amm_json = amm_resp.text().await?;
    let mut amm_file = File::create("raydium_amm_pools.json")?;
    amm_file.write_all(amm_json.as_bytes())?;
    println!("已保存 raydium_amm_pools.json");

    // 拉取Raydium CPMM池子
    let cpmm_resp = reqwest::get("https://api-v3.raydium.io/pools/concentrated/list").await?;
    let cpmm_json = cpmm_resp.text().await?;
    let mut cpmm_file = File::create("raydium_cpmm_pools.json")?;
    cpmm_file.write_all(cpmm_json.as_bytes())?;
    println!("已保存 raydium_cpmm_pools.json");

    // 拉取Pump.fun池子
    let pump_resp = reqwest::get("https://frontend-api.pump.fun/coins").await?;
    let pump_json = pump_resp.text().await?;
    let mut pump_file = File::create("pump_pools.json")?;
    pump_file.write_all(pump_json.as_bytes())?;
    println!("已保存 pump_pools.json");

    Ok(())
} 