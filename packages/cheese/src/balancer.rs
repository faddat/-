// lib/src/balancer.rs
//
// This module encapsulates the entire "price-balancer" logic that was originally in
// price-balancer/src/main.rs. We replicate that feature set and data flow here, so
// that the main file can remain minimal.

use anyhow::{anyhow, Result};
use reqwest::Client;
use tokio::time::{sleep, Duration};

use crate::{
    common::{percent_diff, CHEESE_MINT},
    meteora::{fetch_meteora_cheese_pools, MeteoraPool},
};

/// A simple "wallet" struct to track leftover cheese & stable
#[derive(Debug)]
pub struct Wallet {
    pub leftover_cheese: f64,
    pub leftover_other: f64,
}

/// A smaller struct for storing partial pool data
#[derive(Debug)]
struct CheesePoolPrice {
    pool_address: String,
    pool_name: String,
    price_usd: f64,
    fee_pct: f64,
    tvl: f64,
}

/// The main rebalancing function, replicating all steps from the original price-balancer main
/// (fetch, compute implied prices, rebalancing trades, etc.).
pub async fn run_price_balancer() -> Result<()> {
    let client = Client::new();
    let mut wallet = Wallet {
        leftover_cheese: 10_000.0,
        leftover_other: 0.0,
    };

    // Step 1: fetch all Cheese pools from Meteora
    let all_pools: Vec<MeteoraPool> = fetch_meteora_cheese_pools(&client).await?;
    println!(
        "\nFetched a total of {} Cheese pools from Meteora.\n",
        all_pools.len()
    );

    // Step 2: compute an implied price for each pool
    let mut pool_prices = Vec::new();
    for p in &all_pools {
        let fee_pct = p.total_fee_pct.parse::<f64>().unwrap_or(0.0);

        // placeholder logic for price
        let price_usd = if p.pool_tvl > 0.0 {
            p.pool_tvl / 500.0
        } else {
            0.0
        };

        pool_prices.push(CheesePoolPrice {
            pool_address: p.pool_address.clone(),
            pool_name: p.pool_name.clone(),
            price_usd,
            fee_pct,
            tvl: p.pool_tvl,
        });
    }

    // Step 3: find average price ignoring zeros
    let valid_prices: Vec<f64> = pool_prices
        .iter()
        .filter(|pp| pp.price_usd > 0.0)
        .map(|pp| pp.price_usd)
        .collect();

    let fair_price = if !valid_prices.is_empty() {
        let sum: f64 = valid_prices.iter().sum();
        sum / valid_prices.len() as f64
    } else {
        0.0
    };
    if fair_price == 0.0 {
        println!("No valid price => can't rebalance. Exiting...");
        return Ok(());
    }
    println!("Fair Cheese price is ~ ${:.4}", fair_price);

    // Step 4: Rebalance overpriced/underpriced pools
    for pp in &pool_prices {
        if pp.price_usd == 0.0 {
            continue;
        }
        let diff_pct = percent_diff(pp.price_usd, fair_price);
        if diff_pct <= pp.fee_pct {
            println!(
                "[{}] Price ${:.4}, diff {:.2}%, <= fee {:.2}%, skip",
                pp.pool_name, pp.price_usd, diff_pct, pp.fee_pct
            );
            continue;
        }

        let trade_size_cheese = 100.0;
        if pp.price_usd > fair_price {
            // overpriced => SELL cheese
            println!(
                "[{}] Overpriced by {:.2}%. SELL cheese => leftover stable?",
                pp.pool_name, diff_pct
            );
            if wallet.leftover_cheese >= trade_size_cheese {
                wallet.leftover_cheese -= trade_size_cheese;

                let stable_gained = trade_size_cheese * pp.price_usd;
                // apply fee
                let actual_stable = stable_gained * (1.0 - pp.fee_pct / 100.0);
                wallet.leftover_other += actual_stable;
            } else {
                println!("Not enough cheese to sell for rebalance.");
            }
        } else {
            // underpriced => BUY cheese
            println!(
                "[{}] Underpriced by {:.2}%. BUY cheese => leftover stable spent?",
                pp.pool_name, diff_pct
            );
            // if we had stable, we would spend it. We'll skip in this example
        }
    }

    // Step 5: For pools under $600, deposit Cheese + "other token"
    let mut under_600: Vec<&CheesePoolPrice> =
        pool_prices.iter().filter(|pp| pp.tvl < 600.0).collect();
    under_600.sort_by(|a, b| {
        a.tvl
            .partial_cmp(&b.tvl)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for pp in under_600 {
        let needed = 600.0 - pp.tvl;
        println!(
            "[{}] TVL ${:.2} < $600 => deposit Cheese + other token to raise ~${:.2}",
            pp.pool_name, pp.tvl, needed
        );
        let half_needed = needed / 2.0;
        let cheese_deposit = half_needed / fair_price;
        if wallet.leftover_cheese < cheese_deposit {
            println!("Not enough cheese leftover. Skipping...");
            continue;
        }
        wallet.leftover_cheese -= cheese_deposit;

        if wallet.leftover_other < half_needed {
            println!("Not enough 'other' leftover. Skipping or partial deposit...");
            continue;
        }
        wallet.leftover_other -= half_needed;

        println!(
            " -> Deposited ~{:.2} Cheese & ${:.2} of other => new TVL ~600",
            cheese_deposit, half_needed
        );
    }

    // Final summary
    println!(
        "\nFinal leftover Cheese: {:.2}, leftover Other: {:.2}",
        wallet.leftover_cheese, wallet.leftover_other
    );

    println!("Done balancing & depositing!\n");
    sleep(Duration::from_secs(2)).await;
    Ok(())
}
