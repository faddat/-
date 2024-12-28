use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::de::{self, Deserializer};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use tokio::time::{sleep, Duration};

const CHEESE_MINT: &str = "A3hzGcTxZNSc7744CWB2LR5Tt9VTtEaQYpP6nwripump";

// -----------------------------------
// Part 1: Data Models
// -----------------------------------
#[derive(Debug, Deserialize)]
struct PaginatedPoolSearchResponse {
    data: Vec<MeteoraPool>,
    page: i32,
    total_count: i32,
}

#[derive(Debug, Deserialize)]
struct MeteoraPool {
    pool_address: String,
    pool_name: String,
    pool_token_mints: Vec<String>,
    pool_type: String,
    total_fee_pct: String,

    // We won't read these
    #[allow(dead_code)]
    unknown: bool,
    #[allow(dead_code)]
    permissioned: bool,

    #[serde(deserialize_with = "de_string_to_f64")]
    pool_tvl: f64,

    #[serde(alias = "trading_volume")]
    daily_volume: f64,

    // Add pool_token_amounts so we can show how much CHEESE + other is in the pool
    #[serde(default)]
    pool_token_amounts: Vec<String>,
}

fn de_string_to_f64<'de, D>(deserializer: D) -> std::result::Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    s.parse::<f64>().map_err(de::Error::custom)
}

/// For minted data from Raydium
#[derive(Debug, Deserialize)]
struct RaydiumMintIdsResponse {
    id: String,
    success: bool,
    data: Vec<Option<RaydiumMintItem>>,
}

#[derive(Debug, Deserialize)]
struct RaydiumMintItem {
    #[serde(default)]
    address: String,
    #[serde(default)]
    symbol: String,
}

/// For Raydium cheese pools from /pools/info/mint
#[derive(Debug, Deserialize)]
struct RaydiumMintPoolsResponse {
    id: String,
    success: bool,
    data: RaydiumMintPoolsData,
}

#[derive(Debug, Deserialize)]
struct RaydiumMintPoolsData {
    count: u64,
    data: Vec<RaydiumPoolDetailed>,

    #[allow(non_snake_case)]
    hasNextPage: bool,
}

#[derive(Debug, Deserialize)]
struct RaydiumPoolDetailed {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    programId: String,
    #[serde(default, alias = "id")]
    pool_id: String,

    mintA: RaydiumMintItem,
    mintB: RaydiumMintItem,

    #[serde(default)]
    price: f64,
    #[serde(default, alias = "mintAmountA")]
    mint_amount_a: f64,
    #[serde(default, alias = "mintAmountB")]
    mint_amount_b: f64,
    #[serde(default)]
    feeRate: f64,
    #[serde(default)]
    openTime: String,
    #[serde(default)]
    tvl: f64,
    #[serde(default)]
    day: RaydiumDayStats,
}

#[derive(Debug, Default, Deserialize)]
struct RaydiumDayStats {
    #[serde(default)]
    volume: f64,
}

/// Our final table structure
#[derive(Debug)]
struct DisplayPool {
    source: String, // "Meteora" or "Raydium"
    other_mint: String,
    other_symbol: String,
    cheese_qty: String, // how much Cheese is in the pool
    other_qty: String,  // how much of other asset is in the pool
    pool_type: String,
    liquidity_usd: String,
    volume_usd: String,
    fee: String,
    pool_address: String,
}

// -----------------------------------
// Part 2: Networking
// -----------------------------------
async fn fetch_meteora_cheese_pools(client: &Client) -> Result<Vec<MeteoraPool>> {
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
                ("page".to_string(), page.to_string()),
                ("size".to_string(), size.to_string()),
                ("include_token_mints".to_string(), CHEESE_MINT.to_string()),
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

/// gather all unique mints from the Meteora pools
fn gather_all_mints(meteora: &[MeteoraPool]) -> HashSet<String> {
    let mut set = HashSet::new();
    set.insert(CHEESE_MINT.to_string());
    for pool in meteora {
        for m in &pool.pool_token_mints {
            set.insert(m.clone());
        }
    }
    set
}

async fn fetch_raydium_mint_ids(
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

async fn fetch_raydium_cheese_pools(client: &Client) -> Result<Vec<RaydiumPoolDetailed>> {
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

// -----------------------------------
// Part 3: Main
// -----------------------------------
#[tokio::main]
async fn main() -> Result<()> {
    let client = Client::new();

    // 1) Grab Cheese pools from Meteora
    let meteora_pools = fetch_meteora_cheese_pools(&client).await?;

    // 2) gather all mints from those pools
    let all_mints = gather_all_mints(&meteora_pools);
    let mut all_mints_vec: Vec<String> = all_mints.into_iter().collect();
    all_mints_vec.sort();

    // 3) fetch Raydium minted data (mint -> symbol)
    let minted_data = fetch_raydium_mint_ids(&client, &all_mints_vec).await?;
    let mut mint_to_symbol = HashMap::new();
    for maybe_item in &minted_data {
        if let Some(item) = maybe_item {
            if !item.address.is_empty() {
                mint_to_symbol.insert(item.address.clone(), item.symbol.clone());
            }
        }
    }

    // Convert Meteora -> DisplayPool
    let mut final_pools = Vec::new();
    for pool in &meteora_pools {
        // figure out which is Cheese, which is other
        let (cheese_ix, other_ix) = if pool.pool_token_mints.len() == 2 {
            if pool.pool_token_mints[0] == CHEESE_MINT {
                (0, 1)
            } else {
                (1, 0)
            }
        } else {
            // fallback if there's only 1 or none
            (0, 0)
        };

        let cheese_amt_str = if pool.pool_token_amounts.len() > cheese_ix {
            pool.pool_token_amounts[cheese_ix].clone()
        } else {
            "".to_string()
        };

        let other_mint = if pool.pool_token_mints.len() > other_ix {
            pool.pool_token_mints[other_ix].clone()
        } else {
            "".to_string()
        };
        let other_amt_str = if pool.pool_token_amounts.len() > other_ix {
            pool.pool_token_amounts[other_ix].clone()
        } else {
            "".to_string()
        };

        // If Raydium doesn't have a symbol, fallback to parse from pool_name
        let other_symbol = mint_to_symbol
            .get(&other_mint)
            .cloned()
            .unwrap_or_else(|| parse_other_token_name(&pool.pool_name));

        final_pools.push(DisplayPool {
            source: "Meteora".to_string(),
            other_mint: other_mint,
            other_symbol,
            cheese_qty: cheese_amt_str,
            other_qty: other_amt_str,
            pool_type: pool.pool_type.clone(),
            liquidity_usd: format!("{:.2}", pool.pool_tvl),
            volume_usd: format!("{:.2}", pool.daily_volume),
            fee: pool.total_fee_pct.clone(),
            pool_address: pool.pool_address.clone(),
        });
    }

    // 4) Raydium cheese pools
    let raydium_cheese_pools = fetch_raydium_cheese_pools(&client).await?;
    for rp in &raydium_cheese_pools {
        // figure out which side is Cheese, which is other
        let (cheese_side_amt, other_side_amt, other_mint_addr, other_symbol) =
            if rp.mintA.address == CHEESE_MINT {
                // A is cheese
                let oh_mint = rp.mintB.address.clone();
                let oh_sym = mint_to_symbol
                    .get(&oh_mint)
                    .cloned()
                    .unwrap_or(rp.mintB.symbol.clone());
                (rp.mint_amount_a, rp.mint_amount_b, oh_mint, oh_sym)
            } else {
                // B is cheese
                let oh_mint = rp.mintA.address.clone();
                let oh_sym = mint_to_symbol
                    .get(&oh_mint)
                    .cloned()
                    .unwrap_or(rp.mintA.symbol.clone());
                (rp.mint_amount_b, rp.mint_amount_a, oh_mint, oh_sym)
            };

        let daily_vol = rp.day.volume;
        let tvl = rp.tvl;
        let fee_str = format!("{:.4}", rp.feeRate);

        final_pools.push(DisplayPool {
            source: "Raydium".to_string(),
            other_mint: other_mint_addr,
            other_symbol,
            cheese_qty: format!("{:.2}", cheese_side_amt),
            other_qty: format!("{:.2}", other_side_amt),
            pool_type: rp.r#type.clone(),
            liquidity_usd: format!("{:.2}", tvl),
            volume_usd: format!("{:.2}", daily_vol),
            fee: fee_str,
            pool_address: rp.pool_id.clone(),
        });
    }

    // 5) Print everything
    print_table(&final_pools);

    sleep(Duration::from_secs(2)).await;
    Ok(())
}

// -----------------------------------
// Part 4: Helpers
// -----------------------------------
fn parse_other_token_name(pool_name: &str) -> String {
    let parts: Vec<&str> = pool_name.split('-').collect();
    if parts.len() == 2 {
        let left = parts[0].trim();
        let right = parts[1].trim();
        if left.contains("🧀") || left.to_lowercase().contains("cheese") {
            return right.to_string();
        }
        if right.contains("🧀") || right.to_lowercase().contains("cheese") {
            return left.to_string();
        }
        return right.to_string();
    }
    pool_name.to_string()
}

fn print_table(pools: &[DisplayPool]) {
    // Notice the extra columns: Cheese Qty, Other Qty
    println!(
        "| {:<8} | {:<44} | {:<10} | {:>10} | {:>10} | {:<10} | {:>12} | {:>12} | {:>5} | {:<44} |",
        "Source",
        "Other Mint",
        "Symbol",
        "Cheese Qty",
        "Other Qty",
        "Pool Type",
        "Liquidity($)",
        "Volume($)",
        "Fee",
        "Pool Address",
    );

    println!(
        "|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|",
        "-".repeat(8),
        "-".repeat(44),
        "-".repeat(10),
        "-".repeat(10),
        "-".repeat(10),
        "-".repeat(10),
        "-".repeat(12),
        "-".repeat(12),
        "-".repeat(5),
        "-".repeat(44),
    );

    for dp in pools {
        println!(
            "| {:<8} | {:<44} | {:<10} | {:>10} | {:>10} | {:<10} | {:>12} | {:>12} | {:>5} | {:<44} |",
            dp.source,
            truncate(&dp.other_mint, 44),
            truncate(&dp.other_symbol, 10),
            dp.cheese_qty,
            dp.other_qty,
            truncate(&dp.pool_type, 10),
            dp.liquidity_usd,
            dp.volume_usd,
            dp.fee,
            truncate(&dp.pool_address, 44),
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
