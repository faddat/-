use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::Deserialize;
use tokio::time::{sleep, Duration};

#[derive(Debug, Deserialize)]
struct PoolResponse {
    pool_address: String,
    pool_name: String,
    pool_token_mints: Vec<String>,
    pool_token_amounts: Vec<String>,
    pool_token_usd_amounts: Vec<String>,
    // More fields exist in the OpenAPI spec (e.g. daily_base_apy, total_fee_pct, etc.)
    // Add them here if you need to print or process them.
}

#[tokio::main]
async fn main() -> Result<()> {
    // Create an HTTP client
    let client = Client::new();

    // URL for Meteora's /pools endpoint
    let url = "https://api.meteora.ag/pools";

    // Fetch pools
    println!("Querying Meteora pools at: {}", url);
    let resp = client.get(url).send().await?;

    if !resp.status().is_success() {
        return Err(anyhow!("Meteora API request failed: {}", resp.status()));
    }

    // Parse JSON into a Vec<PoolResponse>
    let pools_data: Vec<PoolResponse> = resp.json().await?;
    println!("Found {} pools.\n", pools_data.len());

    // Print table header
    println!(
        "| {:<44} | {:<25} | {:<30} |",
        "Pool Address", 
        "Pool Name",
        "Token Mints",
    );
    println!(
        "|-{}-|-{}-|-{}-|",
        "-".repeat(44),
        "-".repeat(25),
        "-".repeat(30)
    );

    // Print each pool in a row
    for pool in &pools_data {
        // Join token mints into a single string
        let mints_str = pool.pool_token_mints.join(", ");

        println!(
            "| {:<44} | {:<25} | {:<30} |",
            pool.pool_address,
            truncate(&pool.pool_name, 25),
            truncate(&mints_str, 30),
        );
    }

    // Sleep a bit so you can see the output before the program terminates (if needed)
    sleep(Duration::from_secs(2)).await;
    Ok(())
}

/// Utility: If a field is too long, truncate it for nicer table printing.
fn truncate(input: &str, max_len: usize) -> String {
    if input.len() <= max_len {
        input.to_string()
    } else {
        format!("{}…", &input[..max_len.saturating_sub(1)])
    }
}
