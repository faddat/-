// lib/src/raydium.rs
//
// Logic + data models for Raydium
use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::Deserialize;

use crate::common::CHEESE_MINT;

#[derive(Debug, Deserialize)]
pub struct RaydiumMintPoolsResponse {
    pub id: String,
    pub success: bool,
    pub data: RaydiumMintPoolsData,
}

#[derive(Debug, Deserialize)]
pub struct RaydiumMintPoolsData {
    pub count: u64,
    #[allow(non_snake_case)]
    pub hasNextPage: bool,
    pub data: Vec<RaydiumPoolDetailed>,
}

#[derive(Debug, Deserialize)]
pub struct RaydiumPoolDetailed {
    #[serde(default, alias = "type")]
    pub rtype: String,

    #[serde(default)]
    pub programId: String,

    #[serde(default, alias = "id")]
    pub pool_id: String,

    #[serde(default)]
    pub mintA: RaydiumMintItem,

    #[serde(default)]
    pub mintB: RaydiumMintItem,

    #[serde(default)]
    pub price: f64,
    #[serde(default, alias = "mintAmountA")]
    pub mint_amount_a: f64,
    #[serde(default, alias = "mintAmountB")]
    pub mint_amount_b: f64,

    #[serde(default)]
    pub feeRate: f64,

    #[serde(default)]
    pub openTime: String,

    #[serde(default)]
    pub tvl: f64,

    #[serde(default)]
    pub day: RaydiumDayStats,
}

#[derive(Debug, Deserialize, Default)]
pub struct RaydiumDayStats {
    #[serde(default)]
    pub volume: f64,
}

#[derive(Debug, Deserialize)]
pub struct RaydiumMintItem {
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub symbol: String,
}

/// Fetch Raydium pools containing cheese
pub async fn fetch_raydium_cheese_pools(client: &Client) -> Result<Vec<RaydiumPoolDetailed>> {
    let url = format!(
        "https://api-v3.raydium.io/pools/info/mint?mint1={}&poolType=all&poolSortField=default&sortType=desc&pageSize=1000&page=1",
        CHEESE_MINT
    );
    println!("Requesting Raydium cheese pools from {url}");

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("Raydium request failed: {}", resp.status()));
    }

    let parsed: RaydiumMintPoolsResponse = resp.json().await?;
    if !parsed.success {
        return Err(anyhow!(
            "Raydium cheese-pools returned success=false for id {}",
            parsed.id
        ));
    }

    println!(
        "Raydium /pools/info/mint returned {} items\n",
        parsed.data.count
    );
    Ok(parsed.data.data)
}
