use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::Deserialize;
use tokio::time::{sleep, Duration};

/// Cheese mint on Solana.
const CHEESE_MINT: &str = "A3hzGcTxZNSc7744CWB2LR5Tt9VTtEaQYpP6nwripump";

// According to the OpenAPI spec, each pool inherits from "Metrics" plus additional fields.
// We only define the fields we care about here. 
#[derive(Debug, Deserialize)]
struct PoolResponse {
    pool_address: String,
    pool_name: String,
    pool_token_mints: Vec<String>,
    pool_type: Option<i32>,
    total_fee_pct: String,
    unknown: bool,
    permissioned: bool,

    daily_volume: f64, 
    pool_tvl: f64,
}

/// Same display struct as before for the final Markdown table.
#[derive(Debug)]
struct DisplayPool {
    token_a: String,
    token_b: String,
    pair: String,
    pool_type: String,
    price_type: String,
    liquidity_usd: String,
    volume_usd: String,
    fee: String,
    additional_info: String,
    pool_address: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // 1) Create an HTTP client.
    let client = Client::new();

    // 2) We will use the /pools/search endpoint with query params 
    //    to filter by the Cheese mint.
    let base_url = "https://amm-v2.meteora.ag";
    let search_url = format!("{}/pools/search", base_url);

    // 3) Build our GET request with query parameters:
    //    - page=0, size=50 (so we get up to 50 results)
    //    - include_token_mints = CHEESE_MINT
    println!("Querying Meteora pools that include Cheese at: {}", search_url);

    let resp = client
        .get(&search_url)
        .query(&[
            ("page", "0"),
            ("size", "50"),
            ("include_token_mints", CHEESE_MINT),
        ])
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(anyhow!("Meteora API request failed: {}", resp.status()));
    }

    // 4) Parse JSON into a Vec<PoolResponse>.
    let pools_data: Vec<PoolResponse> = resp.json().await?;
    println!("Found {} pools that contain CHEESE.\n", pools_data.len());

    // 5) Convert them into a "DisplayPool" structure for easy printing.
    let display_pools: Vec<DisplayPool> = pools_data.iter().map(|p| {
        let (token_a, token_b) = match p.pool_token_mints.as_slice() {
            [a, b, ..] => (a.clone(), b.clone()),
            [a]        => (a.clone(), "N/A".to_string()),
            _          => ("N/A".to_string(), "N/A".to_string()),
        };

        let pair_str = format!("{}-{}", token_a, token_b);

        // Map pool_type (just an example mapping)
        let ptype_str = match p.pool_type {
            Some(0) => "dynamic".to_string(),
            Some(1) => "multitoken".to_string(),
            Some(2) => "lst".to_string(),
            Some(3) => "farms".to_string(),
            Some(_) => "other".to_string(),
            None    => "unknown".to_string(),
        };

        // Derive a "price type" guess from the pool name
        let lower_name = p.pool_name.to_lowercase();
        let price_type_str = if lower_name.contains("usdc")
            && (lower_name.contains("usdt") || lower_name.contains("uxd") || lower_name.contains("dai"))
        {
            "Stable".to_string()
        } else {
            if p.unknown {
                "Unknown".to_string()
            } else {
                "Volatile".to_string()
            }
        };

        let liquidity_usd = format!("{:.2}", p.pool_tvl);
        let volume_usd = format!("{:.2}", p.daily_volume);
        let fee_str = p.total_fee_pct.clone();
        let additional_info_str = format!("unknown={}, permissioned={}", p.unknown, p.permissioned);

        DisplayPool {
            token_a,
            token_b,
            pair: pair_str,
            pool_type: ptype_str,
            price_type: price_type_str,
            liquidity_usd,
            volume_usd,
            fee: fee_str,
            additional_info: additional_info_str,
            pool_address: p.pool_address.clone(),
        }
    }).collect();

    // 6) Print a Markdown-style table
    print_table(&display_pools);

    // Optional sleep
    sleep(Duration::from_secs(2)).await;
    Ok(())
}

/// Print the final table in Markdown style
fn print_table(pools: &[DisplayPool]) {
    println!(
        "| {:<12} | {:<12} | {:<25} | {:<10} | {:<10} | {:>12} | {:>12} | {:>5} | {:<25} | {:<44} |",
        "Token A",
        "Token B",
        "Pair",
        "Pool Type",
        "Price Type",
        "Liquidity($)",
        "Volume($)",
        "Fee",
        "Additional Info",
        "Pool Address",
    );

    println!(
        "|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|",
        "-".repeat(12),
        "-".repeat(12),
        "-".repeat(25),
        "-".repeat(10),
        "-".repeat(10),
        "-".repeat(12),
        "-".repeat(12),
        "-".repeat(5),
        "-".repeat(25),
        "-".repeat(44),
    );

    for dp in pools {
        println!(
            "| {:<12} | {:<12} | {:<25} | {:<10} | {:<10} | {:>12} | {:>12} | {:>5} | {:<25} | {:<44} |",
            truncate(&dp.token_a, 12),
            truncate(&dp.token_b, 12),
            truncate(&dp.pair, 25),
            truncate(&dp.pool_type, 10),
            truncate(&dp.price_type, 10),
            dp.liquidity_usd,
            dp.volume_usd,
            dp.fee,
            truncate(&dp.additional_info, 25),
            truncate(&dp.pool_address, 44)
        );
    }
}

fn truncate(input: &str, max_len: usize) -> String {
    if input.len() <= max_len {
        input.to_string()
    } else {
        format!("{}…", &input[..max_len.saturating_sub(1)])
    }
}