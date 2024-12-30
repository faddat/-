use crate::common::{de_string_to_f64, CHEESE_MINT};
use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::Deserialize;

pub async fn fetch_raydium_mint_ids(
    client: &Client,
    mints: &[String],
) -> Result<Vec<Option<RaydiumMintItem>>> {
    let joined = mints.join(",");
    let url = format!("https://api-v3.raydium.io/mint/ids?mints={}", joined);
    println!("Requesting minted data from Raydium for mints: {joined}");

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!(
            "Raydium /mint/ids request failed: {}",
            resp.status()
        ));
    }

    let parsed: RaydiumMintIdsResponse = resp.json().await?;
    if !parsed.success {
        return Err(anyhow!("Raydium /mint/ids returned success=false"));
    }

    println!(
        "Got {} minted items from Raydium (some may be None).",
        parsed.data.len()
    );
    Ok(parsed.data)
}

pub async fn fetch_raydium_cheese_pools(client: &Client) -> Result<Vec<RaydiumPoolDetailed>> {
    // fetch pools for cheese
    let url = format!(
        "https://api-v3.raydium.io/pools/info/mint?mint1={}&poolType=all&poolSortField=default&sortType=desc&pageSize=1000&page=1",
        CHEESE_MINT
    );
    println!("Requesting Raydium cheese pools from {url}");

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!(
            "Raydium cheese-pools request failed: {}",
            resp.status()
        ));
    }

    let parsed: RaydiumMintPoolsResponse = resp.json().await?;
    if !parsed.success {
        return Err(anyhow!("Raydium cheese-pools returned success=false"));
    }

    println!(
        "Raydium /pools/info/mint returned {} items\n",
        parsed.data.count
    );
    Ok(parsed.data.data)
}

/// Raydium mint query
#[derive(Debug, Deserialize)]
pub struct RaydiumMintIdsResponse {
    pub id: String,
    pub success: bool,
    pub data: Vec<Option<RaydiumMintItem>>,
}

#[derive(Debug, Deserialize)]
pub struct RaydiumMintItem {
    pub address: String,
    pub symbol: String,
}

/// Raydium cheese pools
#[derive(Debug, Deserialize)]
pub struct RaydiumMintPoolsResponse {
    pub id: String,
    pub success: bool,
    pub data: RaydiumMintPoolsData,
}

#[derive(Debug, Deserialize)]
pub struct RaydiumMintPoolsData {
    pub count: u64,
    pub data: Vec<RaydiumPoolDetailed>,
    #[allow(non_snake_case)]
    pub hasNextPage: bool,
}

#[derive(Debug, Deserialize)]
pub struct RaydiumPoolDetailed {
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub programId: String,
    #[serde(default, alias = "id")]
    pub pool_id: String,

    pub mintA: RaydiumMintItem,
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

#[derive(Debug, Default, Deserialize)]
pub struct RaydiumDayStats {
    #[serde(default)]
    pub volume: f64,
}
