use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::de::{self, Deserializer};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use tokio::time::{sleep, Duration};

/// The Cheese mint on Solana
const CHEESE_MINT: &str = "A3hzGcTxZNSc7744CWB2LR5Tt9VTtEaQYpP6nwripump";

/// ==========================
/// = Part 1: Data Structures
/// ==========================

// ---------- A) METEORA stuff ----------
#[derive(Debug, Deserialize)]
struct PaginatedPoolSearchResponse {
    data: Vec<MeteoraPool>,
    page: i32,
    total_count: i32,
}

fn de_string_to_f64<'de, D>(deserializer: D) -> std::result::Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    s.parse::<f64>().map_err(de::Error::custom)
}

/// A single pool from Meteora
#[derive(Debug, Deserialize)]
struct MeteoraPool {
    pool_address: String,
    pool_name: String,
    pool_token_mints: Vec<String>,
    pool_type: String,
    total_fee_pct: String,

    // leftover fields
    unknown: bool,
    permissioned: bool,

    #[serde(deserialize_with = "de_string_to_f64")]
    pool_tvl: f64,

    #[serde(alias = "trading_volume")]
    daily_volume: f64,
}

// ---------- B) RAYDIUM minted data (with Option) -----------

#[derive(Debug, Deserialize)]
struct RaydiumMintIdsResponse {
    /// some ID for the response
    id: String,
    success: bool,

    /// ***Key difference:*** data can be `null` for unknown entries
    data: Vec<Option<RaydiumMintItem>>,
}

/// If the entire object can be null, we accept `Option<...>` in the array.
/// Inside each item, some fields might also be missing or null, so use `#[serde(default)]`.
#[derive(Debug, Deserialize)]
struct RaydiumMintItem {
    #[serde(default)]
    chainId: u64,

    #[serde(default)]
    address: String,

    #[serde(default)]
    programId: String,

    #[serde(default)]
    logoURI: String,

    #[serde(default)]
    symbol: String,

    #[serde(default)]
    name: String,

    #[serde(default)]
    decimals: u8,

    #[serde(default)]
    tags: Vec<String>,

    #[serde(default)]
    extensions: HashMap<String, serde_json::Value>,
}

// ---------- C) Final table structure ----------

#[derive(Debug)]
struct DisplayPool {
    other_mint: String,
    other_name: String,
    pool_type: String,
    liquidity_usd: String,
    volume_usd: String,
    fee: String,
    pool_address: String,
    // Raydium minted symbol
    other_symbol: String,
}

/// parse helper for a pool name
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

// ==========================
// = Part 2: HTTP Fetching
// ==========================

/// 1) Fetch Cheese Pools from Meteora
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

        let paginated: PaginatedPoolSearchResponse = resp.json().await?;
        println!(
            "Got {} pools on page {}, total_count={}",
            paginated.data.len(),
            paginated.page,
            paginated.total_count
        );

        all_pools.extend(paginated.data);

        let fetched_so_far = ((page + 1) * size) as i32;
        if fetched_so_far >= paginated.total_count {
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

/// Gather all unique mints (Cheese + “other”) from your Meteora pools
fn gather_all_mints(meteora_pools: &[MeteoraPool]) -> HashSet<String> {
    let mut set = HashSet::new();
    // Always add Cheese
    set.insert(CHEESE_MINT.to_string());

    for pool in meteora_pools {
        for m in &pool.pool_token_mints {
            set.insert(m.clone());
        }
    }
    set
}

/// 2) Request Raydium minted data for all mints in one go
///    with a `Vec<Option<RaydiumMintItem>>` to handle `null`s
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
        "Got {} items (some may be null) from Raydium.\n",
        parsed.data.len()
    );
    Ok(parsed.data)
}

// ==========================
// = Part 3: Main
// ==========================

#[tokio::main]
async fn main() -> Result<()> {
    let client = Client::new();

    // 1) Fetch your Cheese Pools from Meteora
    let meteora_pools = fetch_meteora_cheese_pools(&client).await?;

    // 2) Gather all unique mints
    let all_mints = gather_all_mints(&meteora_pools);
    let mut all_mints_vec: Vec<String> = all_mints.into_iter().collect();
    all_mints_vec.sort();

    // 3) Fetch minted data from Raydium
    let minted_data = fetch_raydium_mint_ids(&client, &all_mints_vec).await?;
    // minted_data is Vec<Option<RaydiumMintItem>>

    // Build a map: mint -> symbol
    let mut mint_to_symbol = HashMap::new();

    // We iterate over each item in `minted_data`.
    // If it’s `Some(item)`, we use item.address + item.symbol.
    // If it’s `None`, that means Raydium returned `null` => we skip or label unknown.
    for maybe_item in &minted_data {
        if let Some(item) = maybe_item {
            // item.address => item.symbol
            // (Raydium always returns item.address as the minted address.)
            mint_to_symbol.insert(item.address.clone(), item.symbol.clone());
        } else {
            // This entry was null => skip or log
        }
    }

    // 4) Convert each Meteora pool into DisplayPool, using the minted_data
    let display_pools: Vec<DisplayPool> = meteora_pools
        .iter()
        .map(|p| {
            // figure out which is Cheese vs. other
            let (cheese_mint_in_pool, other_mint) = if p.pool_token_mints.len() >= 2 {
                if p.pool_token_mints[0] == CHEESE_MINT {
                    (p.pool_token_mints[0].clone(), p.pool_token_mints[1].clone())
                } else {
                    (p.pool_token_mints[1].clone(), p.pool_token_mints[0].clone())
                }
            } else {
                (CHEESE_MINT.to_string(), "???".to_string())
            };

            let other_name = parse_other_token_name(&p.pool_name);

            // Look up a symbol from the map. If missing, use "???".
            let symbol = mint_to_symbol
                .get(&other_mint)
                .cloned()
                .unwrap_or_else(|| "???".to_string());

            DisplayPool {
                other_mint: other_mint.clone(),
                other_name,
                pool_type: p.pool_type.clone(),
                liquidity_usd: format!("{:.2}", p.pool_tvl),
                volume_usd: format!("{:.2}", p.daily_volume),
                fee: p.total_fee_pct.clone(),
                pool_address: p.pool_address.clone(),
                other_symbol: symbol,
            }
        })
        .collect();

    // 5) Print your final table
    print_table(&display_pools);

    // let user see the results
    sleep(Duration::from_secs(2)).await;
    Ok(())
}

/// Print table, we add a "Symbol" column
fn print_table(pools: &[DisplayPool]) {
    println!(
        "| {:<44} | {:<8} | {:<10} | {:>12} | {:>12} | {:>5} | {:<44} |",
        "Other Mint", "Symbol", "Pool Type", "Liquidity($)", "Volume($)", "Fee", "Pool Address",
    );

    println!(
        "|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|",
        "-".repeat(44),
        "-".repeat(8),
        "-".repeat(10),
        "-".repeat(12),
        "-".repeat(12),
        "-".repeat(5),
        "-".repeat(44),
    );

    for dp in pools {
        println!(
            "| {:<44} | {:<8} | {:<10} | {:>12} | {:>12} | {:>5} | {:<44} |",
            truncate(&dp.other_mint, 44),
            truncate(&dp.other_symbol, 8),
            truncate(&dp.pool_type, 10),
            dp.liquidity_usd,
            dp.volume_usd,
            dp.fee,
            truncate(&dp.pool_address, 44),
        );
    }
}

/// Utility
fn truncate(input: &str, max_len: usize) -> String {
    if input.len() <= max_len {
        input.to_string()
    } else {
        format!("{}…", &input[..max_len.saturating_sub(1)])
    }
}
