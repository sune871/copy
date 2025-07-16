use serde::Deserialize;
use std::fs;

#[derive(Debug, Deserialize, Clone)]
pub struct RaydiumAmmPool {
    pub id: String,
    pub base_mint: String,
    pub quote_mint: String,
    pub lp_mint: Option<String>,
    pub market_id: Option<String>,
    pub program_id: Option<String>,
    // 其它字段可按需扩展
}

#[derive(Debug, Deserialize, Clone)]
pub struct RaydiumCpmmPool {
    pub id: String,
    pub mint_a: String,
    pub mint_b: String,
    pub vault_a: String,
    pub vault_b: String,
    pub program_id: Option<String>,
    // 其它字段可按需扩展
}

#[derive(Debug, Deserialize, Clone)]
pub struct PumpPool {
    pub mint: String,
    pub program_id: Option<String>,
    // 其它字段可按需扩展
}

pub struct PoolLoader {
    pub raydium_amm: Vec<RaydiumAmmPool>,
    pub raydium_cpmm: Vec<RaydiumCpmmPool>,
    pub pump: Vec<PumpPool>,
}

impl PoolLoader {
    pub fn load() -> Self {
        let raydium_amm = fs::read_to_string("raydium_amm_pools.json")
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let raydium_cpmm = fs::read_to_string("raydium_cpmm_pools.json")
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let pump = fs::read_to_string("pump_pools.json")
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        PoolLoader { raydium_amm, raydium_cpmm, pump }
    }

    pub fn find_amm_by_pool(&self, pool_id: &str) -> Option<&RaydiumAmmPool> {
        self.raydium_amm.iter().find(|p| p.id == pool_id)
    }
    pub fn find_cpmm_by_pool(&self, pool_id: &str) -> Option<&RaydiumCpmmPool> {
        self.raydium_cpmm.iter().find(|p| p.id == pool_id)
    }
    pub fn find_pump_by_mint(&self, mint: &str) -> Option<&PumpPool> {
        self.pump.iter().find(|p| p.mint == mint)
    }
}

// 你可以在主程序中这样用：
// let loader = PoolLoader::load();
// let pool = loader.find_amm_by_pool("池子地址"); 