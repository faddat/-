// lib/src/lib.rs
use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::de::{self, Deserializer};
use serde::Deserialize;
use std::collections::HashSet;

pub const CHEESE_MINT: &str = "A3hzGcTxZNSc7744CWB2LR5Tt9VTtEaQYpP6nwripump";

// ---------- Data Models from your prior code ----------

#[derive(Debug, Deserialize)]
pub struct PaginatedResponse {
    pub data: Vec<PoolInfo>,
    pub page: i32,
    pub total_count: i32,
}

#[derive(Debug, Deserialize)]
pub struct PoolInfo {
    pub pool_address: String,
    pub pool_name: String,
    pub pool_token_mints: Vec<String>,
    pub pool_type: String,
    pub total_fee_pct: String,
    pub unknown: bool,
    pub permissioned: bool,

    #[serde(deserialize_with = "de_string_to_f64")]
    pub pool_tvl: f64,

    #[serde(alias = "trading_volume")]
    pub daily_volume: f64,

    #[serde(default)]
    pub pool_token_amounts: Vec<String>,
}

// Helper so we can share it
fn de_string_to_f64<'de, D>(deserializer: D) -> std::result::Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    s.parse().map_err(de::Error::custom)
}

// ---------- Shared logic: fetch_meteora_cheese_pools ----------

pub async fn fetch_meteora_cheese_pools(client: &Client) -> Result<Vec<PoolInfo>> {
    let base_url = "https://amm-v2.meteora.ag/pools/search";

    let mut all = Vec::new();
    let mut page = 0;
    let size = 50;

    loop {
        println!("Fetching page {} from Meteora...", page);
        let resp = client
            .get(base_url)
            .query(&[
                ("page", page.to_string()),
                ("size", size.to_string()),
                ("include_token_mints", CHEESE_MINT.to_string()),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow!(
                "Meteora /pools/search request failed: {}",
                resp.status()
            ));
        }

        let parsed: PaginatedResponse = resp.json().await?;
        println!(
            "Got {} pools on page {}, total_count={}",
            parsed.data.len(),
            parsed.page,
            parsed.total_count
        );

        all.extend(parsed.data);

        let fetched_so_far = ((page + 1) * size) as i32;
        if fetched_so_far >= parsed.total_count {
            break;
        }
        page += 1;
    }

    println!(
        "\nFetched a total of {} Cheese pools from Meteora.\n",
        all.len()
    );
    Ok(all)
}

// Example shared helper: returns the absolute diff % between two numbers
pub fn percent_diff(a: f64, b: f64) -> f64 {
    if (a + b) == 0.0 {
        0.0
    } else {
        ((a - b).abs() * 200.0) / (a + b)
    }
}

// More shared code can be added here: data mgitodels for Raydium, fetchers, etc.
