use anyhow::Result;
use libcheese::common::parse_other_token_name;
use libcheese::common::CHEESE_MINT;
use libcheese::jupiter::fetch_jupiter_prices;
use libcheese::meteora::fetch_meteora_cheese_pools;
use libcheese::meteora::MeteoraPool;
use libcheese::raydium::fetch_raydium_cheese_pools;
use libcheese::raydium::fetch_raydium_mint_ids;
use reqwest::Client;
use std::collections::{HashMap, HashSet};

/// Our final table row
#[derive(Debug)]
struct DisplayPool {
    source: String, // "Meteora" or "Raydium"
    other_mint: String,
    other_symbol: String,
    cheese_qty: String,
    other_qty: String,
    /// The "pool_type" from Meteora or the "type" from Raydium
    pool_type: String,
    /// The total liquidity in USD
    liquidity_usd: String,
    /// 24H volume in USD (or daily volume)
    volume_usd: String,
    /// The pool fee (like "0.25" or "6"), stored as string
    fee: String,
    /// The pool address
    pool_address: String,

    /// ***New Field***: The implied Cheese price, e.g. "$1.23"
    cheese_price: String,
}

// Additional stats about Cheese
#[derive(Debug, Default)]
struct CheeseAggregates {
    total_liquidity_usd: f64,
    total_trades_all_time: u64,
    total_cheese_qty: f64,
    total_volume_24h: f64,
}

/// gather all unique mints
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

// -----------------------------------
// Main
// -----------------------------------
#[tokio::main]
async fn main() -> Result<()> {
    let client = Client::new();

    // 1) fetch cheese pools from Meteora
    let meteora_pools = fetch_meteora_cheese_pools(&client).await?;

    // 2) gather mints from those pools
    let all_mints = gather_all_mints(&meteora_pools);
    // Convert to a sorted Vec for convenience
    let mut all_mints_vec: Vec<String> = all_mints.into_iter().collect();
    all_mints_vec.sort();

    // 3) fetch minted data from Raydium to get minted symbols, etc.
    let minted_data = fetch_raydium_mint_ids(&client, &all_mints_vec).await?;
    let mut mint_to_symbol = HashMap::new();
    for maybe_item in &minted_data {
        if let Some(item) = maybe_item {
            if !item.address.is_empty() {
                mint_to_symbol.insert(item.address.clone(), item.symbol.clone());
            }
        }
    }

    // *** NEW ***: Call Jupiter to get last-swapped prices
    let jup_prices = fetch_jupiter_prices(&client, &all_mints_vec).await?;

    // 4) transform Meteora => DisplayPool
    let mut cheese_aggs = CheeseAggregates::default();
    let mut final_pools = Vec::new();

    for pool in &meteora_pools {
        // figure out which side is Cheese, which is other
        let (cheese_ix, other_ix) = if pool.pool_token_mints.len() == 2 {
            if pool.pool_token_mints[0] == CHEESE_MINT {
                (0, 1)
            } else {
                (1, 0)
            }
        } else {
            (0, 0)
        };

        let cheese_amt_str = pool
            .pool_token_amounts
            .get(cheese_ix)
            .cloned()
            .unwrap_or_default();
        let cheese_amt_f64 = cheese_amt_str.parse::<f64>().unwrap_or(0.0);

        let other_mint = pool
            .pool_token_mints
            .get(other_ix)
            .cloned()
            .unwrap_or_default();
        let other_amt_str = pool
            .pool_token_amounts
            .get(other_ix)
            .cloned()
            .unwrap_or_default();
        let other_amt_f64 = other_amt_str.parse::<f64>().unwrap_or(0.0);

        // Attempt to fetch Jupiter price of CHEESE, or fallback to half_TVL method
        let half_tvl = pool.pool_tvl / 2.0;
        let fallback_price = if cheese_amt_f64 > 0.0 {
            half_tvl / cheese_amt_f64
        } else {
            0.0
        };

        // If you want the Jupiter price for Cheese itself, we might do:
        let cheese_jup_price = jup_prices
            .get(CHEESE_MINT)
            .cloned()
            .unwrap_or(fallback_price);

        // Then do the same if you want the "other" token’s Jupiter price
        // e.g. let other_jup_price = jup_prices.get(&other_mint).unwrap_or(???)

        // We’ll store the CHEESE price in the table as either Jupiter’s or fallback
        let chosen_cheese_price = cheese_jup_price;

        // fallback symbol
        let other_symbol = mint_to_symbol
            .get(&other_mint)
            .cloned()
            .unwrap_or_else(|| parse_other_token_name(&pool.pool_name));

        // Accumulate aggregates
        cheese_aggs.total_liquidity_usd += pool.pool_tvl;
        cheese_aggs.total_volume_24h += pool.daily_volume;
        cheese_aggs.total_cheese_qty += cheese_amt_f64;
        cheese_aggs.total_trades_all_time += 1; // placeholder

        final_pools.push(DisplayPool {
            source: "Meteora".to_string(),
            other_mint,
            other_symbol,
            cheese_qty: format!("{:.2}", cheese_amt_f64),
            other_qty: format!("{:.2}", other_amt_f64),
            pool_type: pool.pool_type.clone(),
            liquidity_usd: format!("{:.2}", pool.pool_tvl),
            volume_usd: format!("{:.2}", pool.daily_volume),
            fee: pool.total_fee_pct.clone(),
            pool_address: pool.pool_address.clone(),
            cheese_price: format!("${:.8}", chosen_cheese_price),
        });
    }

    // ...

    // after building `final_pools` from meteora, do:
    let raydium_pools = fetch_raydium_cheese_pools(&client).await?;
    for rp in &raydium_pools {
        // figure out which side is cheese
        let (cheese_side_amt, other_side_amt, cheese_mint_addr, other_mint_addr) =
            if rp.mintA.address == CHEESE_MINT {
                (
                    rp.mint_amount_a,
                    rp.mint_amount_b,
                    rp.mintA.address.clone(),
                    rp.mintB.address.clone(),
                )
            } else {
                (
                    rp.mint_amount_b,
                    rp.mint_amount_a,
                    rp.mintB.address.clone(),
                    rp.mintA.address.clone(),
                )
            };

        // Then compute a fallback price if you want half-of-tvl / cheese
        let half_tvl = rp.tvl / 2.0;
        let fallback_price = if cheese_side_amt > 0.0 {
            half_tvl / cheese_side_amt
        } else {
            0.0
        };

        // Then get Jupiter’s price for Cheese:
        let cheese_jup = jup_prices
            .get(CHEESE_MINT)
            .cloned()
            .unwrap_or(fallback_price);

        // Or if you want to do something else (like fetch the other token’s Jupiter price
        // and compute ratio), that’s up to you.

        // Then push to final_pools
        final_pools.push(DisplayPool {
            source: "Raydium".to_string(),
            other_mint: other_mint_addr.clone(),
            other_symbol: mint_to_symbol
                .get(&other_mint_addr)
                .cloned()
                .unwrap_or_else(|| "???".to_string()),
            cheese_qty: format!("{:.2}", cheese_side_amt),
            other_qty: format!("{:.2}", other_side_amt),
            pool_type: rp.r#type.clone(),
            liquidity_usd: format!("{:.2}", rp.tvl),
            volume_usd: format!("{:.2}", rp.day.volume),
            fee: format!("{:.4}", rp.feeRate),
            pool_address: rp.pool_id.clone(),
            cheese_price: format!("${:.6}", cheese_jup), // up to 6 decimals
        });
    }

    // 5) fetch Raydium cheese pools
    // ... same approach ...
    // also possibly use cheese_jup_price from jup_prices if you like

    // Print aggregator stats
    println!("===== Cheese Aggregates =====");
    // ... etc ...
    print_table(&final_pools);

    Ok(())
}

/// Print the table with a new "Cheese Price" column
fn print_table(pools: &[DisplayPool]) {
    println!(
        "| {:<8} | {:<44} | {:<10} | {:>10} | {:>10} | {:<10} | {:>12} | {:>12} | {:>5} | {:>11} | {:<44} |",
        "Source",
        "Other Mint",
        "Symbol",
        "Cheese Qty",
        "Other Qty",
        "Pool Type",
        "Liquidity($)",
        "Volume($)",
        "Fee",
        "CheesePrice",
        "Pool Address",
    );

    println!(
        "|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|-{}-|",
        "-".repeat(8),
        "-".repeat(44),
        "-".repeat(10),
        "-".repeat(10),
        "-".repeat(10),
        "-".repeat(10),
        "-".repeat(12),
        "-".repeat(12),
        "-".repeat(5),
        "-".repeat(11),
        "-".repeat(44),
    );

    for dp in pools {
        println!(
            "| {:<8} | {:<44} | {:<10} | {:>10} | {:>10} | {:<10} | {:>12} | {:>12} | {:>5} | {:>11} | {:<44} |",
            dp.source,
            truncate(&dp.other_mint, 44),
            truncate(&dp.other_symbol, 10),
            dp.cheese_qty,
            dp.other_qty,
            truncate(&dp.pool_type, 10),
            dp.liquidity_usd,
            dp.volume_usd,
            dp.fee,
            dp.cheese_price,
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
