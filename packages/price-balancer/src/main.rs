use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::de::{self, Deserializer};
use serde::Deserialize;
use tokio::time::{sleep, Duration};

/// The Cheese mint on Solana
const CHEESE_MINT: &str = "A3hzGcTxZNSc7744CWB2LR5Tt9VTtEaQYpP6nwripump";

/// Meteora's paginated response
#[derive(Debug, Deserialize)]
struct PaginatedResponse {
    data: Vec<PoolInfo>,
    page: i32,
    total_count: i32,
}

// For fields that may be numeric strings
fn de_string_to_f64<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    s.parse().map_err(de::Error::custom)
}

/// Pool info from Meteora
#[derive(Debug, Deserialize)]
struct PoolInfo {
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

    #[serde(default)]
    pool_token_amounts: Vec<String>,
    #[serde(default)]
    pool_token_prices: Vec<f64>,
}

/// For storing partial pool data with implied price
#[derive(Debug)]
struct CheesePoolPrice {
    pool_address: String,
    pool_name: String,
    price_usd: f64,
    fee_pct: f64,
    tvl: f64, // to identify < $600
}

/// A simple wallet struct
#[derive(Debug)]
struct Wallet {
    leftover_cheese: f64,
    leftover_other: f64, // Possibly we accumulate other tokens if we rebalanced
}

#[tokio::main]
async fn main() -> Result<()> {
    let client = Client::new();
    let base_url = "https://amm-v2.meteora.ag/pools/search";

    // Move wallet outside the loop to persist balance between iterations
    let mut wallet = Wallet {
        leftover_cheese: 10000.0,
        leftover_other: 0.0,
    };

    loop {
        println!("\n=== Starting new price balancing iteration ===");

        let mut all_pools = Vec::new();
        let mut page = 0;
        let size = 50;

        // Step 1: fetch all Cheese pools
        loop {
            println!("Fetching page {}...", page);
            let resp = client
                .get(base_url)
                .query(&[
                    ("page", page.to_string()),
                    ("size", size.to_string()),
                    ("include_token_mints", CHEESE_MINT.to_string()),
                ])
                .send()
                .await?;

            if !resp.status().is_success() {
                return Err(anyhow!("API request failed: {}", resp.status()));
            }

            let data: PaginatedResponse = resp.json().await?;
            println!("Got {} pools on page {}", data.data.len(), data.page);

            all_pools.extend(data.data);

            let fetched_so_far = (page + 1) * size;
            if fetched_so_far as i32 >= data.total_count {
                break;
            }
            page += 1;
        }

        println!(
            "\nFetched a total of {} Cheese pools from Meteora.\n",
            all_pools.len()
        );

        // Step 2: compute an implied price for each pool
        let mut pool_prices = Vec::new();
        for p in &all_pools {
            let fee_pct = p.total_fee_pct.parse::<f64>().unwrap_or(0.0);

            // Identify which index belongs to Cheese, which to "other"
            // Only proceed if we have exactly 2 tokens, else fallback
            let (cheese_ix, other_ix) = if p.pool_token_mints.len() == 2 {
                if p.pool_token_mints[0] == CHEESE_MINT {
                    (0, 1)
                } else if p.pool_token_mints[1] == CHEESE_MINT {
                    (1, 0)
                } else {
                    // This pool doesn't actually have Cheese, fallback
                    (usize::MAX, usize::MAX)
                }
            } else {
                // Not a 2-token pool
                (usize::MAX, usize::MAX)
            };

            // New approach: check if we can parse an implied price from pool_token_amounts & pool_token_prices
            let price_usd = if cheese_ix < p.pool_token_mints.len()
                && other_ix < p.pool_token_mints.len()
                && p.pool_token_amounts.len() == 2
                && p.pool_token_prices.len() == 2
            {
                // parse amounts
                let cheese_amt: f64 = p.pool_token_amounts[cheese_ix].parse().unwrap_or(0.0);
                let other_amt: f64 = p.pool_token_amounts[other_ix].parse().unwrap_or(0.0);

                // parse prices
                let cheese_price = p.pool_token_prices[cheese_ix];
                let other_price = p.pool_token_prices[other_ix];

                if cheese_amt > 0.0 && other_price > 0.0 {
                    // implied cheese price = (other_amt * other_price) / cheese_amt
                    (other_amt * other_price) / cheese_amt
                } else {
                    // fallback if data missing or zero
                    if p.pool_tvl > 0.0 {
                        p.pool_tvl / 500.0
                    } else {
                        0.0
                    }
                }
            } else {
                // fallback: old approach if no good data or not exactly 2 tokens
                if p.pool_tvl > 0.0 {
                    p.pool_tvl / 500.0 // a rough guess
                } else {
                    0.0
                }
            };

            println!(
                "[{}] Pool price: ${:.4} (TVL: ${:.2})",
                p.pool_name, price_usd, p.pool_tvl
            );

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

        // After calculating fair price, add more detailed output
        if fair_price > 0.0 {
            println!("\nPrice analysis:");
            println!("Fair price: ${:.4}", fair_price);

            // Calculate price statistics
            let min_price = valid_prices.iter().fold(f64::INFINITY, |a, &b| a.min(b));
            let max_price = valid_prices
                .iter()
                .fold(f64::NEG_INFINITY, |a, &b| a.max(b));

            println!("Price range: ${:.4} - ${:.4}", min_price, max_price);
            println!("Number of valid pools: {}", valid_prices.len());
            println!("Price spread: {:.2}%\n", percent_diff(min_price, max_price));
        }

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

            // Decide a small trade
            let trade_size_cheese = 100.0;
            if pp.price_usd > fair_price {
                // overpriced => sell cheese
                println!(
                    "[{}] Overpriced by {:.2}%. SELL cheese => leftover stable?",
                    pp.pool_name, diff_pct
                );
                if wallet.leftover_cheese >= trade_size_cheese {
                    wallet.leftover_cheese -= trade_size_cheese;

                    let stable_gained = trade_size_cheese * pp.price_usd;
                    // apply fee
                    let actual_stable = stable_gained * (1.0 - pp.fee_pct / 100.0);
                    // we store it as leftover_other in this example
                    wallet.leftover_other += actual_stable;
                } else {
                    println!("Not enough cheese to sell for rebalance.");
                }
            } else {
                // underpriced => buy cheese
                println!(
                    "[{}] Underpriced by {:.2}%. BUY cheese => leftover stable spent?",
                    pp.pool_name, diff_pct
                );
                // if we had stable, we could spend it. But let's skip in this example
            }
        }

        // Step 5: For pools under $600, deposit Cheese + "other token"
        // We'll find the ones with tvl < 600, sorted ascending
        let mut under_600: Vec<&CheesePoolPrice> =
            pool_prices.iter().filter(|pp| pp.tvl < 600.0).collect();
        under_600.sort_by(|a, b| {
            a.tvl
                .partial_cmp(&b.tvl)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // We deposit in ascending order
        for pp in under_600 {
            // figure out how much we need to deposit to bring it up to $600
            let needed = 600.0 - pp.tvl;
            if needed <= 0.0 {
                continue;
            }
            println!(
                "[{}] TVL ${:.2} < $600 => deposit Cheese + other token to raise ~${:.2}",
                pp.pool_name, pp.tvl, needed
            );
            // In reality, you’d do a real deposit: we pair Cheese with the "unstable" asset
            // We assume we convert leftover_other to that "unstable" asset if needed.

            // For demonstration, let's deposit half Cheese, half "other"
            // So half in Cheese => needed/2 / price => how many cheese we deposit
            let half_needed = needed / 2.0;
            let cheese_deposit = half_needed / fair_price;
            if wallet.leftover_cheese < cheese_deposit {
                println!("Not enough cheese leftover to deposit in this pool. Skipping...");
                continue;
            }
            wallet.leftover_cheese -= cheese_deposit;

            // We also need an "other" deposit => let's see if leftover_other is enough
            // For demonstration, assume leftover_other is in USD value or convertible at 1:1
            if wallet.leftover_other < half_needed {
                println!("Not enough 'other' leftover to deposit. Skipping or partial deposit...");
                continue;
            }
            wallet.leftover_other -= half_needed;

            // We pretend we've deposited. TVL is ~600 now
            println!(
                " -> Deposited ~{:.2} Cheese & ${:.2} of other. Pool is near $600 now!",
                cheese_deposit, half_needed
            );
        }

        // Final summary
        println!(
            "\nFinal leftover Cheese: {:.2}, leftover Other: {:.2}",
            wallet.leftover_cheese, wallet.leftover_other
        );

        println!("Done balancing & depositing!");
        sleep(Duration::from_secs(2)).await;

        println!("\nSleeping for 30 seconds before next iteration...\n");
        sleep(Duration::from_secs(30)).await;
    }
}

/// Return the absolute difference as a percentage of their average
fn percent_diff(a: f64, b: f64) -> f64 {
    if (a + b) == 0.0 {
        0.0
    } else {
        ((a - b).abs() * 200.0) / (a + b)
    }
}
