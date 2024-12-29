// lib/src/meteora.rs
//
// Logic + data models for interacting with Meteora
use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::Deserialize;

use crate::common::{de_string_to_f64, CHEESE_MINT};

#[derive(Debug, Deserialize)]
pub struct PaginatedPoolSearchResponse {
    pub data: Vec<MeteoraPool>,
    pub page: i32,
    pub total_count: i32,
}

#[derive(Debug, Deserialize)]
pub struct MeteoraPool {
    pub pool_address: String,
    pub pool_name: String,
    pub pool_token_mints: Vec<String>,
    pub pool_type: String,
    pub total_fee_pct: String,

    #[allow(dead_code)]
    pub unknown: bool,
    #[allow(dead_code)]
    pub permissioned: bool,

    #[serde(deserialize_with = "de_string_to_f64")]
    pub pool_tvl: f64,

    #[serde(alias = "trading_volume")]
    pub daily_volume: f64,

    #[serde(default)]
    pub pool_token_amounts: Vec<String>,
}

/// Fetch all Cheese pools from the Meteora endpoint
pub async fn fetch_meteora_cheese_pools(client: &Client) -> Result<Vec<MeteoraPool>> {
    let base_url = "https://amm-v2.meteora.ag";
    let search_url = format!("{}/pools/search", base_url);

    let mut all_pools = Vec::new();
    let mut page = 0;
    let size = 50;

    loop {
        println!("Requesting page {page} from {search_url}");
        let resp = client
            .get(&search_url)
            .query(&[
                ("page", page.to_string()),
                ("size", size.to_string()),
                ("include_token_mints", CHEESE_MINT.to_string()),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow!("Meteora request failed: {}", resp.status()));
        }

        let parsed: PaginatedPoolSearchResponse = resp.json().await?;
        println!(
            "Got {} pools on page {}, total_count={}",
            parsed.data.len(),
            parsed.page,
            parsed.total_count
        );

        all_pools.extend(parsed.data);

        let fetched_so_far = ((page + 1) * size) as i32;
        if fetched_so_far >= parsed.total_count {
            break;
        }
        page += 1;
    }

    println!(
        "\nFetched a total of {} Cheese pools from Meteora.\n",
        all_pools.len()
    );
    Ok(all_pools)
}
