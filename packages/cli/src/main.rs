use anyhow::Result;
use libcheese::common::{parse_other_token_name, CHEESE_MINT};
use libcheese::jupiter::fetch_jupiter_prices;
use libcheese::meteora::fetch_meteora_cheese_pools;
use libcheese::raydium::{fetch_raydium_cheese_pools, fetch_raydium_mint_ids};
use reqwest::Client;
use std::collections::{HashMap, HashSet};

const WALLET_CHEESE_BALANCE: f64 = 5_000_000.0;
const WALLET_SOL_BALANCE: f64 = 1.0;
const SOL_PER_TX: f64 = 0.000005; // Approximate SOL cost per transaction

/// A row describing one pool
#[derive(Debug)]
struct DisplayPool {
    source: String,
    other_mint: String,
    other_symbol: String,
    cheese_qty: String,
    other_qty: String,
    pool_type: String,
    tvl: String,
    volume_usd: String,
    fee: String,
    pool_address: String,
    cheese_price: String, // e.g. "$0.000057"
}

#[derive(Debug, Default)]
struct CheeseAggregates {
    total_liquidity_usd: f64,
    number_of_pools: u64,
    total_cheese_qty: f64,
    total_volume_24h: f64,
}

#[derive(Debug, Clone)]
struct PoolEdge {
    pool_address: String,
    source: String, // "Meteora" or "Raydium"
    token_a: String,
    token_b: String,
    fee: f64,
    tvl: f64,
    reserves_a: f64,
    reserves_b: f64,
}

#[derive(Debug)]
struct ArbitrageCycle {
    steps: Vec<TradeStep>,
    initial_cheese: f64,
    final_cheese: f64,
    total_fees_sol: f64,
    pool_fees_paid: Vec<f64>,
    initial_usdc_value: f64, // Value of initial CHEESE in USDC
    final_usdc_value: f64,   // Value of final CHEESE in USDC
    fees_usdc_value: f64,    // Value of all fees in USDC
}

#[derive(Debug, Clone)]
struct TradeStep {
    pool_address: String,
    source: String,
    sell_token: String,
    buy_token: String,
    amount_in: f64,
    expected_out: f64,
    fee_percent: f64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let client = Client::new();

    // 1) fetch from Meteora
    let meteora_pools = fetch_meteora_cheese_pools(&client).await?;

    // gather unique mints
    let mut set = HashSet::new();
    set.insert(CHEESE_MINT.to_string());
    for pool in &meteora_pools {
        for m in &pool.pool_token_mints {
            set.insert(m.clone());
        }
    }
    let mut all_mints_vec: Vec<String> = set.into_iter().collect();
    all_mints_vec.sort();

    // fetch minted data from Raydium
    let minted_data = fetch_raydium_mint_ids(&client, &all_mints_vec).await?;
    let mut mint_to_symbol = HashMap::new();
    for maybe_item in &minted_data {
        if let Some(item) = maybe_item {
            mint_to_symbol.insert(item.address.clone(), item.symbol.clone());
        }
    }

    // fetch Jupiter prices
    let jup_prices = fetch_jupiter_prices(&client, &all_mints_vec).await?;

    // Find the USDC/CHEESE price from the specific pool
    let usdc_pool = meteora_pools
        .iter()
        .find(|p| p.pool_address == "2rkTh46zo8wUvPJvACPTJ16RNUHEM9EZ1nLYkUxZEHkw")
        .unwrap();
    let (cheese_ix, usdc_ix) = if usdc_pool.pool_token_mints[0] == CHEESE_MINT {
        (0, 1)
    } else {
        (1, 0)
    };
    let cheese_amt: f64 = usdc_pool.pool_token_amounts[cheese_ix]
        .parse()
        .unwrap_or(0.0);
    let usdc_amt: f64 = usdc_pool.pool_token_amounts[usdc_ix].parse().unwrap_or(0.0);
    let cheese_usdc_price = if cheese_amt > 0.0 {
        usdc_amt / cheese_amt
    } else {
        0.0
    };

    // aggregator
    let mut cheese_aggs = CheeseAggregates::default();
    let mut final_pools = Vec::new();

    // ---- PART A: Meteora pools ----
    for pool in &meteora_pools {
        println!("\nProcessing Meteora pool: {}", pool.pool_address);
        // figure out cheese vs. other
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

        println!("  Cheese amount: {}", cheese_amt_f64);
        println!("  Other amount: {}", other_amt_f64);
        println!("  Cheese USDC price: {}", cheese_usdc_price);
        println!(
            "  Other token price from Jupiter: {}",
            jup_prices.get(&other_mint).cloned().unwrap_or(0.0)
        );

        // Calculate total liquidity using both sides of the pool
        let cheese_side = cheese_amt_f64 * cheese_usdc_price;
        let other_side = other_amt_f64 * jup_prices.get(&other_mint).cloned().unwrap_or(0.0);
        let pool_tvl = cheese_side + other_side;

        println!("  Cheese side value: ${:.2}", cheese_side);
        println!("  Other side value: ${:.2}", other_side);
        println!("  Total pool TVL: ${:.2}", pool_tvl);

        // ratio approach
        let other_jup_price = jup_prices.get(&other_mint).cloned().unwrap_or(0.0);
        let ratio_price = if cheese_amt_f64 > 0.0 {
            (other_amt_f64 * other_jup_price) / cheese_amt_f64
        } else {
            0.0
        };

        // Use USDC price instead of fallback
        let universal = cheese_usdc_price;

        // final chosen
        let chosen_cheese_price = if ratio_price > 0.0 {
            ratio_price
        } else {
            universal
        };

        let other_symbol = mint_to_symbol
            .get(&other_mint)
            .cloned()
            .unwrap_or_else(|| parse_other_token_name(&pool.pool_name));

        // If symbol is unknown, try to get it from Jupiter prices
        let other_symbol = if other_symbol == "???" {
            if let Some(price) = jup_prices.get(&other_mint) {
                // If Jupiter knows about this token, try to parse from Meteora pools
                if let Some(meteora_pool) = meteora_pools
                    .iter()
                    .find(|p| p.pool_token_mints.contains(&other_mint))
                {
                    parse_other_token_name(&meteora_pool.pool_name)
                } else {
                    other_mint
                        .chars()
                        .rev()
                        .take(4)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect()
                }
            } else {
                "???".to_string()
            }
        } else {
            other_symbol
        };

        // aggregator
        cheese_aggs.total_liquidity_usd += pool_tvl;
        cheese_aggs.total_volume_24h += pool.daily_volume;
        cheese_aggs.total_cheese_qty += cheese_amt_f64;
        cheese_aggs.number_of_pools += 1;

        final_pools.push(DisplayPool {
            source: "Meteora".to_string(),
            other_mint: other_mint,
            other_symbol,
            cheese_qty: format!("{:.2}", cheese_amt_f64),
            other_qty: format!("{:.2}", other_amt_f64),
            pool_type: pool.pool_type.clone(),
            tvl: format!("{:.10}", pool_tvl),
            volume_usd: format!("{:.10}", pool.daily_volume),
            fee: pool.total_fee_pct.clone(),
            pool_address: pool.pool_address.clone(),
            cheese_price: format!("${:.10}", chosen_cheese_price),
        });
    }

    // ---- PART B: Raydium pools ----
    let raydium_pools = fetch_raydium_cheese_pools(&client).await?;
    for rp in &raydium_pools {
        println!("\nProcessing Raydium pool: {}", rp.pool_id);
        let (cheese_amt, other_amt, _, other_mint_addr) = if rp.mintA.address == CHEESE_MINT {
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

        println!("  Cheese amount: {}", cheese_amt);
        println!("  Other amount: {}", other_amt);
        println!("  Cheese USDC price: {}", cheese_usdc_price);
        println!(
            "  Other token price from Jupiter: {}",
            jup_prices.get(&other_mint_addr).cloned().unwrap_or(0.0)
        );

        // Calculate total liquidity using both sides of the pool
        let cheese_side = cheese_amt * cheese_usdc_price;
        let other_side = other_amt * jup_prices.get(&other_mint_addr).cloned().unwrap_or(0.0);
        let pool_tvl = cheese_side + other_side;

        println!("  Cheese side value: ${:.2}", cheese_side);
        println!("  Other side value: ${:.2}", other_side);
        println!("  Total pool TVL: ${:.2}", pool_tvl);

        // Get the symbol from Raydium pool data
        let other_symbol = if rp.mintA.address == CHEESE_MINT {
            rp.mintB.symbol.clone()
        } else {
            rp.mintA.symbol.clone()
        };

        // ratio approach
        let other_jup_price = jup_prices.get(&other_mint_addr).cloned().unwrap_or(0.0);
        let ratio_price = if cheese_amt > 0.0 {
            (other_amt * other_jup_price) / cheese_amt
        } else {
            0.0
        };

        // Use USDC price instead of fallback
        let universal = cheese_usdc_price;

        let chosen_cheese_price = if ratio_price > 0.0 {
            ratio_price
        } else {
            universal
        };

        // If symbol is unknown, try to get it from Jupiter prices or parse from Meteora pool name
        let other_symbol = if other_symbol == "???" {
            // First try Jupiter prices
            if let Some(price) = jup_prices.get(&other_mint_addr) {
                // If Jupiter knows about this token, try to parse from Meteora pools
                if let Some(meteora_pool) = meteora_pools
                    .iter()
                    .find(|p| p.pool_token_mints.contains(&other_mint_addr))
                {
                    parse_other_token_name(&meteora_pool.pool_name)
                } else {
                    other_mint_addr
                        .chars()
                        .rev()
                        .take(4)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect()
                }
            } else {
                "???".to_string()
            }
        } else {
            other_symbol
        };

        // aggregator
        cheese_aggs.total_liquidity_usd += pool_tvl;
        cheese_aggs.total_volume_24h += rp.day.volume;
        cheese_aggs.total_cheese_qty += cheese_amt;
        cheese_aggs.number_of_pools += 1;

        final_pools.push(DisplayPool {
            source: "Raydium".to_string(),
            other_mint: other_mint_addr,
            other_symbol,
            cheese_qty: format!("{:.2}", cheese_amt),
            other_qty: format!("{:.2}", other_amt),
            pool_type: rp.r#type.clone(),
            tvl: format!("{:.10}", pool_tvl),
            volume_usd: format!("{:.10}", rp.day.volume),
            fee: format!("{:.2}", rp.feeRate * 100.0),
            pool_address: rp.pool_id.clone(),
            cheese_price: format!("${:.10}", chosen_cheese_price),
        });
    }

    // Print aggregator stats
    println!("\n===== 🧀 Aggregates =====");
    println!(
        "Total Liquidity (USD):   ${:.2}",
        cheese_aggs.total_liquidity_usd
    );
    println!(
        "Total 24H Volume (USD): ${:.2}",
        cheese_aggs.total_volume_24h
    );
    println!("Number of pools:        {}", cheese_aggs.number_of_pools);
    println!(
        "Total 🧀 in pools:      {:.2}",
        cheese_aggs.total_cheese_qty
    );
    println!("===========================\n");

    // Sort pools by cheese quantity in descending order
    final_pools.sort_by(|a, b| {
        let a_qty = a.cheese_qty.parse::<f64>().unwrap_or(0.0);
        let b_qty = b.cheese_qty.parse::<f64>().unwrap_or(0.0);
        b_qty
            .partial_cmp(&a_qty)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Then print table
    print_table(&final_pools);

    // *** ARBITRAGE LOGIC ***: find differences over $20
    display_arbitrage_opportunities(&final_pools, &jup_prices);

    Ok(())
}

/// After we've built `final_pools`, we look for price differences between Meteora pools
fn display_arbitrage_opportunities(pools: &[DisplayPool], jup_prices: &HashMap<String, f64>) {
    // Filter to only Meteora pools
    let meteora_pools: Vec<&DisplayPool> = pools.iter().filter(|p| p.source == "Meteora").collect();

    // Get CHEESE/USDC price from the USDC/CHEESE pool for reference
    let usdc_pool = pools
        .iter()
        .find(|p| p.pool_address == "2rkTh46zo8wUvPJvACPTJ16RNUHEM9EZ1nLYkUxZEHkw")
        .expect("USDC pool not found");
    let cheese_usdc_price = usdc_pool
        .cheese_price
        .trim_start_matches('$')
        .parse::<f64>()
        .unwrap_or(0.0);

    // Get SOL price for fee calculation
    let sol_price = jup_prices
        .get("So11111111111111111111111111111111111111112")
        .copied()
        .unwrap_or(0.0);

    println!("\n=== Meteora Pool Arbitrage Analysis ===");
    println!("Reference CHEESE/USDC price: ${:.10}", cheese_usdc_price);
    println!("SOL price: ${:.2}", sol_price);
    println!(
        "Transaction cost: ${:.4} ({}◎ per tx)\n",
        SOL_PER_TX * sol_price,
        SOL_PER_TX
    );

    // Calculate implied CHEESE prices from each pool
    let mut pool_prices: Vec<(String, String, f64, f64, f64, f64)> = Vec::new();
    // (pool_addr, symbol, cheese_qty, other_qty, implied_price, fee_percent)

    for pool in &meteora_pools {
        let cheese_qty = pool.cheese_qty.parse::<f64>().unwrap_or(0.0);
        let other_qty = pool.other_qty.parse::<f64>().unwrap_or(0.0);
        let fee_percent = pool.fee.trim_end_matches('%').parse::<f64>().unwrap_or(0.0) / 100.0;

        if cheese_qty <= 0.0 || other_qty <= 0.0 {
            continue;
        }

        // Get the other token's price, either from Jupiter or derive it
        let other_price = if let Some(&price) = jup_prices.get(&pool.other_mint) {
            price
        } else {
            // If we don't have a Jupiter price, derive it from the USDC pool ratio
            (cheese_qty * cheese_usdc_price) / other_qty
        };

        // Calculate implied CHEESE price from this pool
        let implied_cheese_price = (other_qty * other_price) / cheese_qty;

        pool_prices.push((
            pool.pool_address.clone(),
            pool.other_symbol.clone(),
            cheese_qty,
            other_qty,
            implied_cheese_price,
            fee_percent,
        ));
    }

    // Sort by price difference from USDC pool
    pool_prices.sort_by(|a, b| {
        let diff_a = (a.4 - cheese_usdc_price).abs();
        let diff_b = (b.4 - cheese_usdc_price).abs();
        diff_b
            .partial_cmp(&diff_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    println!("\nPotential Arbitrage Opportunities (sorted by price difference):");
    println!("Reference USDC pool price: ${:.10}\n", cheese_usdc_price);

    // Get USDC pool fee
    let usdc_pool_fee = usdc_pool
        .fee
        .trim_end_matches('%')
        .parse::<f64>()
        .unwrap_or(0.0)
        / 100.0;

    for (pool_addr, symbol, cheese_qty, other_qty, implied_price, pool_fee) in &pool_prices {
        let price_diff_pct = ((implied_price - cheese_usdc_price) / cheese_usdc_price) * 100.0;

        // Only show opportunities with >1% difference
        if price_diff_pct.abs() > 1.0 {
            // Calculate total transaction costs
            let tx_cost_usd = 2.0 * SOL_PER_TX * sol_price; // Two transactions needed

            // Calculate optimal trade size considering fees and slippage
            let max_trade_size = if implied_price > &cheese_usdc_price {
                // Selling CHEESE in this pool
                cheese_qty * 0.1 // Limit to 10% of pool liquidity
            } else {
                // Buying CHEESE from this pool
                cheese_qty * 0.1
            };

            // Calculate total fees for the trade
            let pool1_fee_usd = max_trade_size * cheese_usdc_price * usdc_pool_fee;
            let pool2_fee_usd = max_trade_size * implied_price * pool_fee;
            let total_fees_usd = tx_cost_usd + pool1_fee_usd + pool2_fee_usd;

            // Calculate expected profit
            let price_diff_per_cheese = (implied_price - cheese_usdc_price).abs();
            let gross_profit = max_trade_size * price_diff_per_cheese;
            let net_profit = gross_profit - total_fees_usd;

            if net_profit > 0.0 {
                println!("Pool: {} ({})", pool_addr, symbol);
                println!("├─ Implied CHEESE price: ${:.10}", implied_price);
                println!("├─ Price difference: {:.2}%", price_diff_pct);
                println!("├─ Pool liquidity:");
                println!("│  ├─ 🧀: {:.2}", cheese_qty);
                println!("│  └─ {}: {:.2}", symbol, other_qty);
                println!("├─ Fees:");
                println!("│  ├─ Pool 1 (USDC) fee: {:.2}%", usdc_pool_fee * 100.0);
                println!("│  ├─ Pool 2 fee: {:.2}%", pool_fee * 100.0);
                println!("│  └─ Transaction cost: ${:.4}", tx_cost_usd);

                if implied_price > &cheese_usdc_price {
                    println!("├─ Arbitrage strategy (SELL in this pool):");
                    println!(
                        "│  1. Buy {:.2} CHEESE from USDC pool at ${:.10}",
                        max_trade_size, cheese_usdc_price
                    );
                    println!("│  2. Sell in this pool at ${:.10}", implied_price);
                } else {
                    println!("├─ Arbitrage strategy (BUY from this pool):");
                    println!(
                        "│  1. Buy {:.2} CHEESE from this pool at ${:.10}",
                        max_trade_size, implied_price
                    );
                    println!("│  2. Sell in USDC pool at ${:.10}", cheese_usdc_price);
                }

                println!("└─ Profitability:");
                println!("   ├─ Gross profit: ${:.4}", gross_profit);
                println!("   ├─ Total fees: ${:.4}", total_fees_usd);
                println!("   └─ Net profit: ${:.4}\n", net_profit);
            }
        }
    }
}

fn find_arbitrage_cycles(
    edges: &[PoolEdge],
    cheese_usdc_price: f64,
    token_prices: &HashMap<String, (f64, String)>,
) -> Vec<ArbitrageCycle> {
    let mut cycles = Vec::new();
    let mut visited = HashSet::new();
    let mut current_path = Vec::new();

    // Start from CHEESE edges
    for edge in edges.iter() {
        if edge.token_a == CHEESE_MINT {
            visited.clear();
            current_path.clear();
            dfs_find_cycles(
                edge,
                edges,
                &mut visited,
                &mut current_path,
                &mut cycles,
                0,
                WALLET_CHEESE_BALANCE / 10.0,
                cheese_usdc_price,
                token_prices,
            );
        }
    }

    cycles
}

fn dfs_find_cycles(
    current: &PoolEdge,
    edges: &[PoolEdge],
    visited: &mut HashSet<String>,
    path: &mut Vec<TradeStep>,
    cycles: &mut Vec<ArbitrageCycle>,
    depth: usize,
    amount_in: f64,
    cheese_usdc_price: f64,
    token_prices: &HashMap<String, (f64, String)>,
) {
    if depth >= 4 {
        return; // Limit cycle length
    }

    // Skip pools with low TVL
    if current.tvl < 10.0 {
        return;
    }

    // Calculate expected output using constant product formula
    let fee = current.fee;
    let k = current.reserves_a * current.reserves_b;

    // Skip if reserves are too low
    if current.reserves_a <= 0.0 || current.reserves_b <= 0.0 || k <= 0.0 {
        return;
    }

    // Ensure amount_in is not too large relative to reserves
    let max_input = current.reserves_a * 0.3; // Max 30% of reserves
    let amount_in = amount_in.min(max_input);

    let amount_with_fee = amount_in * (1.0 - fee);

    // Calculate output amount using constant product formula
    let new_reserve_a = current.reserves_a + amount_with_fee;
    let new_reserve_b = k / new_reserve_a;
    let expected_out = current.reserves_b - new_reserve_b;

    // Skip if output is invalid
    if expected_out.is_nan() || expected_out <= 0.0 {
        return;
    }

    // Skip if amounts don't match between steps
    if !path.is_empty() {
        let prev_step = &path[path.len() - 1];
        if (prev_step.expected_out - amount_in).abs() > 0.000001 * prev_step.expected_out {
            return;
        }
    }

    // Calculate USD values for validation
    let amount_in_usd = if current.token_a == CHEESE_MINT {
        amount_in * cheese_usdc_price
    } else {
        amount_in
            * token_prices
                .get(&current.token_a)
                .map(|(price, _)| *price)
                .unwrap_or(0.0)
    };

    let amount_out_usd = if current.token_b == CHEESE_MINT {
        expected_out * cheese_usdc_price
    } else {
        expected_out
            * token_prices
                .get(&current.token_b)
                .map(|(price, _)| *price)
                .unwrap_or(0.0)
    };

    // Skip if USD values don't make sense (accounting for fees)
    if amount_out_usd < amount_in_usd * (1.0 - fee * 2.0) {
        return;
    }

    // Add current step
    path.push(TradeStep {
        pool_address: current.pool_address.clone(),
        source: current.source.clone(),
        sell_token: current.token_a.clone(),
        buy_token: current.token_b.clone(),
        amount_in,
        expected_out,
        fee_percent: fee,
    });

    visited.insert(current.pool_address.clone());

    // If we're back to CHEESE and have made at least 2 trades, we found a cycle
    if depth > 0 && current.token_b == CHEESE_MINT {
        // Verify the entire cycle
        let mut valid = true;
        let mut current_amount = path[0].amount_in;
        for step in path.iter() {
            if (step.amount_in - current_amount).abs() > 0.000001 * current_amount {
                valid = false;
                break;
            }
            current_amount = step.expected_out;
        }

        if !valid {
            path.pop();
            visited.remove(&current.pool_address);
            return;
        }

        let total_fees_sol = path.len() as f64 * SOL_PER_TX;
        let pool_fees: Vec<f64> = path.iter().map(|step| step.fee_percent).collect();

        // Calculate USDC values using token prices
        let initial_usdc_value = if path[0].sell_token == CHEESE_MINT {
            path[0].amount_in * cheese_usdc_price
        } else {
            path[0].amount_in
                * token_prices
                    .get(&path[0].sell_token)
                    .map(|(price, _)| *price)
                    .unwrap_or(0.0)
        };

        let final_usdc_value = expected_out * cheese_usdc_price;

        // Calculate fees in USDC
        let sol_price = token_prices
            .get("So11111111111111111111111111111111111111112")
            .map(|(price, _)| *price)
            .unwrap_or(0.0);
        let fees_usdc_value = total_fees_sol * sol_price;

        // Calculate pool fees in USDC
        let pool_fees_usdc: f64 = path
            .iter()
            .map(|step| {
                let step_amount_usd = if step.sell_token == CHEESE_MINT {
                    step.amount_in * cheese_usdc_price
                } else {
                    step.amount_in
                        * token_prices
                            .get(&step.sell_token)
                            .map(|(price, _)| *price)
                            .unwrap_or(0.0)
                };
                step_amount_usd * step.fee_percent
            })
            .sum();

        let total_fees_usdc = fees_usdc_value + pool_fees_usdc;
        let profit_usdc = final_usdc_value - initial_usdc_value - total_fees_usdc;

        // Only add cycle if profit is significant
        if profit_usdc > 1.0 {
            cycles.push(ArbitrageCycle {
                steps: path.clone(),
                initial_cheese: amount_in,
                final_cheese: expected_out,
                total_fees_sol,
                pool_fees_paid: pool_fees,
                initial_usdc_value,
                final_usdc_value,
                fees_usdc_value: total_fees_usdc,
            });
        }
    } else {
        // Continue exploring with the output amount
        for next_edge in edges {
            if next_edge.token_a == current.token_b
                && !visited.contains(&next_edge.pool_address)
                && next_edge.tvl >= 10.0
            {
                dfs_find_cycles(
                    next_edge,
                    edges,
                    visited,
                    path,
                    cycles,
                    depth + 1,
                    expected_out,
                    cheese_usdc_price,
                    token_prices,
                );
            }
        }
    }

    visited.remove(&current.pool_address);
    path.pop();
}

/// Print table
fn print_table(pools: &[DisplayPool]) {
    println!(
        "| {:<8} | {:<44} | {:<10} | {:>10} | {:>10} | {:<10} | {:>20} | {:>20} | {:>5} | {:>19} | {:<44} |",
        "Source",
        "Other Mint",
        "Symbol",
        "🧀 Qty",
        "Other Qty",
        "Pool Type",
        "TVL($)",
        "Volume($)",
        "Fee",
        "🧀 Price",
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
        "-".repeat(20),
        "-".repeat(20),
        "-".repeat(5),
        "-".repeat(19),
        "-".repeat(44),
    );

    for dp in pools {
        println!(
            "| {:<8} | {:<44} | {:<10} | {:>10} | {:>10} | {:<10} | {:>20} | {:>20} | {:>5} | {:>19} | {:<44} |",
            dp.source,
            truncate(&dp.other_mint, 44),
            truncate(&dp.other_symbol, 10),
            dp.cheese_qty,
            dp.other_qty,
            truncate(&dp.pool_type, 10),
            dp.tvl,
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
