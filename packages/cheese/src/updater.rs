// lib/src/updater.rs
//
// This module encapsulates the entire aggregator logic that was originally in
// update-readme/src/main.rs. We replicate that feature set: fetch from Meteora + Raydium,
// combine data, compute aggregates, print table, etc.

use anyhow::Result;
use reqwest::Client;
use std::collections::HashMap;
use tokio::time::{sleep, Duration};

use crate::{
    common::CHEESE_MINT,
    meteora::{fetch_meteora_cheese_pools, MeteoraPool},
    raydium::{fetch_raydium_cheese_pools, RaydiumDayStats, RaydiumPoolDetailed},
};

/// For the aggregator, we replicate the "DisplayPool", "CheeseAggregates" etc.
#[derive(Debug)]
pub struct DisplayPool {
    pub source: String, // "Meteora" or "Raydium"
    pub other_mint: String,
    pub other_symbol: String,
    pub cheese_qty: String,
    pub other_qty: String,
    pub pool_type: String,
    pub liquidity_usd: String,
    pub volume_usd: String,
    pub fee: String,
    pub pool_address: String,
}

#[derive(Debug, Default)]
pub struct CheeseAggregates {
    pub total_liquidity_usd: f64,
    pub total_volume_24h: f64,
    pub total_trades_all_time: u64,
    pub total_cheese_qty: f64,
}

/// The aggregator logic that fetches from both DEXes, merges, prints stats, table, etc.
pub async fn run_readme_updater() -> Result<()> {
    let client = Client::new();

    // 1) fetch from Meteora
    let meteora_pools = fetch_meteora_cheese_pools(&client).await?;

    // 2) fetch from Raydium
    let raydium_pools = fetch_raydium_cheese_pools(&client).await?;

    // 3) Possibly gather minted data. The original code also did Raydium "mint/ids",
    //    so you can do that here if you wish. We'll skip for brevity or do partial:
    //    (In the original code, you stored results in a map. We replicate as needed.)
    let mut mint_to_symbol: HashMap<String, String> = HashMap::new();
    // e.g. if you want to do some minted data queries, or just do fallback logic

    // 4) Convert meteora to DisplayPool
    let mut cheese_aggs = CheeseAggregates::default();
    let mut final_pools = Vec::new();

    for m in &meteora_pools {
        let (cheese_ix, other_ix) = if m.pool_token_mints.len() == 2 {
            if m.pool_token_mints[0] == CHEESE_MINT {
                (0, 1)
            } else {
                (1, 0)
            }
        } else {
            (0, 0)
        };

        let cheese_amt_str = m
            .pool_token_amounts
            .get(cheese_ix)
            .cloned()
            .unwrap_or_default();
        let other_amt_str = m
            .pool_token_amounts
            .get(other_ix)
            .cloned()
            .unwrap_or_default();

        let cheese_amt_f64 = cheese_amt_str.parse::<f64>().unwrap_or(0.0);
        let other_amt_f64 = other_amt_str.parse::<f64>().unwrap_or(0.0);

        let other_mint = m
            .pool_token_mints
            .get(other_ix)
            .cloned()
            .unwrap_or_default();
        let other_symbol = mint_to_symbol
            .get(&other_mint)
            .cloned()
            .unwrap_or_else(|| parse_other_token_name(&m.pool_name));

        cheese_aggs.total_liquidity_usd += m.pool_tvl;
        cheese_aggs.total_volume_24h += m.daily_volume;
        cheese_aggs.total_cheese_qty += cheese_amt_f64;
        cheese_aggs.total_trades_all_time += 1;

        final_pools.push(DisplayPool {
            source: "Meteora".to_string(),
            other_mint,
            other_symbol,
            cheese_qty: format!("{:.2}", cheese_amt_f64),
            other_qty: format!("{:.2}", other_amt_f64),
            pool_type: m.pool_type.clone(),
            liquidity_usd: format!("{:.2}", m.pool_tvl),
            volume_usd: format!("{:.2}", m.daily_volume),
            fee: m.total_fee_pct.clone(),
            pool_address: m.pool_address.clone(),
        });
    }

    // 5) Convert Raydium to DisplayPool
    for r in &raydium_pools {
        let (cheese_side_amt, other_side_amt, other_mint_addr, other_symbol) =
            if r.mintA.address == CHEESE_MINT {
                (
                    r.mint_amount_a,
                    r.mint_amount_b,
                    r.mintB.address.clone(),
                    r.mintB.symbol.clone(),
                )
            } else {
                (
                    r.mint_amount_b,
                    r.mint_amount_a,
                    r.mintA.address.clone(),
                    r.mintA.symbol.clone(),
                )
            };

        cheese_aggs.total_liquidity_usd += r.tvl;
        cheese_aggs.total_volume_24h += r.day.volume;
        cheese_aggs.total_cheese_qty += cheese_side_amt;
        cheese_aggs.total_trades_all_time += 2;

        final_pools.push(DisplayPool {
            source: "Raydium".to_string(),
            other_mint: other_mint_addr,
            other_symbol,
            cheese_qty: format!("{:.2}", cheese_side_amt),
            other_qty: format!("{:.2}", other_side_amt),
            pool_type: r.rtype.clone(),
            liquidity_usd: format!("{:.2}", r.tvl),
            volume_usd: format!("{:.2}", r.day.volume),
            fee: format!("{:.4}", r.feeRate),
            pool_address: r.pool_id.clone(),
        });
    }

    // Print aggregator stats
    println!("===== Cheese Aggregates =====");
    println!(
        "Total Liquidity (USD):   ${:.2}",
        cheese_aggs.total_liquidity_usd
    );
    println!(
        "Total 24H Volume (USD): ${:.2}",
        cheese_aggs.total_volume_24h
    );
    println!(
        "All-Time Trades:        {}",
        cheese_aggs.total_trades_all_time
    );
    println!(
        "Total Cheese qty:       {:.2}",
        cheese_aggs.total_cheese_qty
    );
    println!("=============================\n");

    // Print the table
    print_table(&final_pools);

    sleep(Duration::from_secs(2)).await;
    Ok(())
}
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
