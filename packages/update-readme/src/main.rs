use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::Deserialize;
use serde::de::{self, Deserializer};
use tokio::time::{sleep, Duration};

// The mint for Cheese
const CHEESE_MINT: &str = "A3hzGcTxZNSc7744CWB2LR5Tt9VTtEaQYpP6nwripump";

#[derive(Debug, Deserialize)]
struct PaginatedPoolSearchResponse {
    data: Vec<PoolResponse>,
    page: i32,
    total_count: i32,
}

// Helper for numeric fields that might be strings
fn de_string_to_f64<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    s.parse::<f64>().map_err(de::Error::custom)
}

// Each pool in "data"
#[derive(Debug, Deserialize)]
struct PoolResponse {
    pool_address: String,
    pool_name: String,
    pool_token_mints: Vec<String>,
    pool_type: String,
    total_fee_pct: String,
    unknown: bool,
    permissioned: bool,
    #[serde(deserialize_with = "de_string_to_f64")]
    pool_tvl: f64, 
    #[serde(alias = "trading_volume")]
    daily_volume: f64,
}

/// Our final structure for printing
#[derive(Debug)]
struct DisplayPool {
    cheese_mint: String,
    cheese_name: String,
    other_mint: String,
    other_name: String,
    pool_type: String,
    liquidity_usd: String,
    volume_usd: String,
    fee: String,
    additional_info: String,
    pool_address: String,
}

/// Attempt to parse the token name in the pool name that is *not* 🧀 or Cheese.
/// e.g. "🧀-Ross" => "Ross"
///      "Bonk-🧀" => "Bonk"
///      "CHEESE-USDC" => "USDC"
fn parse_other_token_name(pool_name: &str) -> String {
    // First, split by dash, e.g. "🧀-Ross" => ["🧀", "Ross"]
    // or "Bonk-🧀" => ["Bonk", "🧀"]
    let parts: Vec<&str> = pool_name.split('-').collect();
    if parts.len() == 2 {
        let left = parts[0].trim();
        let right = parts[1].trim();

        // If left is "🧀" or "CHEESE", then return right
        if left.contains("🧀") || left.to_lowercase().contains("cheese") {
            return right.to_string();
        }
        // If right is "🧀" or "CHEESE", then return left
        if right.contains("🧀") || right.to_lowercase().contains("cheese") {
            return left.to_string();
        }
        // If neither side explicitly mentions 🧀, return right
        // or you can do some other default
        return right.to_string();
    }

    // fallback: if no dash or more than one dash, just return the entire pool_name
    pool_name.to_string()
}

#[tokio::main]
async fn main() -> Result<()> {
    let client = Client::new();
    let base_url = "https://amm-v2.meteora.ag";
    let search_url = format!("{}/pools/search", base_url);

    // We'll fetch all pages in a loop
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
            return Err(anyhow!("Meteora API request failed: {}", resp.status()));
        }

        let paginated: PaginatedPoolSearchResponse = resp.json().await?;
        println!(
            "Got {} pools on page {}, total_count={}",
            paginated.data.len(),
            paginated.page,
            paginated.total_count
        );

        all_pools.extend(paginated.data);

        // if we've fetched all pages, break
        let fetched_so_far = ((page + 1) * size) as i32;
        if fetched_so_far >= paginated.total_count {
            break;
        }
        page += 1;
    }

    println!(
        "\nFetched a total of {} Cheese pools across pages.\n",
        all_pools.len()
    );

    // Convert to DisplayPool for printing
    let display_pools: Vec<DisplayPool> = all_pools.iter().map(|p| {
        // figure out which mint is cheese, which is other
        // if pool_token_mints[0] == CHEESE_MINT => then [1] is the other
        // else if pool_token_mints[1] == CHEESE_MINT => [0] is the other
        let (cheese_mint, other_mint) = if p.pool_token_mints.len() >= 2 {
            if p.pool_token_mints[0] == CHEESE_MINT {
                (p.pool_token_mints[0].clone(), p.pool_token_mints[1].clone())
            } else {
                (p.pool_token_mints[1].clone(), p.pool_token_mints[0].clone())
            }
        } else {
            // fallback if there's only 1 mint or none
            (CHEESE_MINT.to_string(), "???".to_string())
        };

        // parse the "other name" from pool_name
        let other_name = parse_other_token_name(&p.pool_name);

        // "Cheese name" is just "🧀", or "Cheese" if you prefer
        let cheese_name = "🧀".to_string();

        let ptype_str = p.pool_type.clone();

        let liquidity_usd = format!("{:.2}", p.pool_tvl);
        let volume_usd = format!("{:.2}", p.daily_volume);
        let fee_str = p.total_fee_pct.clone();
        let additional_info_str =
            format!("unknown={}, permissioned={}", p.unknown, p.permissioned);

        DisplayPool {
            cheese_mint,
            cheese_name,
            other_mint,
            other_name,
            pool_type: ptype_str,
            liquidity_usd,
            volume_usd,
            fee: fee_str,
            additional_info: additional_info_str,
            pool_address: p.pool_address.clone(),
        }
    }).collect();

    // Print table
    print_table(&display_pools);

    sleep(Duration::from_secs(2)).await;
    Ok(())
}

/// Print a Markdown table showing:
/// - Cheese Mint, Cheese Name
/// - Other Mint, Other Name
/// - Liquidity, Volume, Fee, Additional Info
fn print_table(pools: &[DisplayPool]) {
    println!(
        "| {:<44} | {:<10} | {:<44} | {:<10} | {:<10} | {:>12} | {:>12} | {:>5} | {:<25} |",
        "Cheese Mint",
        "Cheese Name",
        "Other Mint",
        "Other Name",
        "Pool Type",
        "Liquidity($)",
        "Volume($)",
        "Fee",
        "Additional Info",
    );
    println!(
        "|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|",
        "-".repeat(44),
        "-".repeat(10),
        "-".repeat(44),
        "-".repeat(10),
        "-".repeat(10),
        "-".repeat(12),
        "-".repeat(12),
        "-".repeat(5),
        "-".repeat(25),
    );

    for dp in pools {
        println!(
            "| {:<44} | {:<10} | {:<44} | {:<10} | {:<10} | {:>12} | {:>12} | {:>5} | {:<25} |",
            truncate(&dp.cheese_mint, 44),
            truncate(&dp.cheese_name, 10),
            truncate(&dp.other_mint, 44),
            truncate(&dp.other_name, 10),
            truncate(&dp.pool_type, 10),
            dp.liquidity_usd,
            dp.volume_usd,
            dp.fee,
            truncate(&dp.additional_info, 25)
        );
    }
}

/// Utility: truncate strings so columns line up
fn truncate(input: &str, max_len: usize) -> String {
    if input.len() <= max_len {
        input.to_string()
    } else {
        format!("{}…", &input[..max_len.saturating_sub(1)])
    }
}