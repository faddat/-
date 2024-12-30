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
            tvl: format!("{:.2}", pool_tvl),
            volume_usd: format!("{:.2}", pool.daily_volume),
            fee: pool.total_fee_pct.clone(),
            pool_address: pool.pool_address.clone(),
            cheese_price: format!("${:.6}", chosen_cheese_price),
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
            tvl: format!("{:.2}", pool_tvl),
            volume_usd: format!("{:.2}", rp.day.volume),
            fee: format!("{:.2}", rp.feeRate * 100.0),
            pool_address: rp.pool_id.clone(),
            cheese_price: format!("${:.8}", chosen_cheese_price),
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

/// After we've built `final_pools`, we look for price differences and calculate equalization value
fn display_arbitrage_opportunities(pools: &[DisplayPool], jup_prices: &HashMap<String, f64>) {
    // Build trading graph
    let mut edges: Vec<PoolEdge> = Vec::new();
    let mut token_symbols = HashMap::new();

    // Convert DisplayPools into PoolEdges
    for pool in pools {
        let fee = pool.fee.trim_end_matches('%').parse::<f64>().unwrap_or(0.0) / 100.0;
        let tvl = pool.tvl.parse::<f64>().unwrap_or(0.0);
        let cheese_qty = pool.cheese_qty.parse::<f64>().unwrap_or(0.0);
        let other_qty = pool.other_qty.parse::<f64>().unwrap_or(0.0);

        // Store symbol mapping
        token_symbols.insert(pool.other_mint.clone(), pool.other_symbol.clone());
        token_symbols.insert(CHEESE_MINT.to_string(), "🧀".to_string());

        // Create bidirectional edges for both CHEESE and the other token
        edges.push(PoolEdge {
            pool_address: pool.pool_address.clone(),
            source: pool.source.clone(),
            token_a: CHEESE_MINT.to_string(),
            token_b: pool.other_mint.clone(),
            fee,
            tvl,
            reserves_a: cheese_qty,
            reserves_b: other_qty,
        });

        edges.push(PoolEdge {
            pool_address: pool.pool_address.clone(),
            source: pool.source.clone(),
            token_a: pool.other_mint.clone(),
            token_b: CHEESE_MINT.to_string(),
            fee,
            tvl,
            reserves_a: other_qty,
            reserves_b: cheese_qty,
        });
    }

    // Get CHEESE/USDC price from the first pool (assuming it's set correctly)
    let cheese_usdc_price = pools
        .first()
        .map(|p| {
            p.cheese_price
                .trim_start_matches('$')
                .parse::<f64>()
                .unwrap_or(0.0)
        })
        .unwrap_or(0.0);

    // Build a map of derived prices from CHEESE pools
    let mut derived_prices = HashMap::new();
    for pool in pools {
        let cheese_qty = pool.cheese_qty.parse::<f64>().unwrap_or(0.0);
        let other_qty = pool.other_qty.parse::<f64>().unwrap_or(0.0);

        if cheese_qty > 0.0 && other_qty > 0.0 {
            let derived_price = (cheese_qty * cheese_usdc_price) / other_qty;
            derived_prices.insert(pool.other_mint.clone(), derived_price);
        }
    }

    // Debug print all prices
    println!("\n=== Token Prices ===");
    for (mint, price) in jup_prices.iter() {
        if let Some(symbol) = token_symbols.get(mint) {
            println!("{}: ${:.6} (Jupiter)", symbol, price);
        }
    }
    for (mint, price) in derived_prices.iter() {
        if let Some(symbol) = token_symbols.get(mint) {
            if !jup_prices.contains_key(mint) {
                println!("{}: ${:.6} (Derived from 🧀)", symbol, price);
            }
        }
    }
    println!("==================\n");

    println!("\n=== Potential Arbitrage Opportunities ===");

    // Find cycles starting from each token
    let mut all_cycles = Vec::new();

    for edge in &edges {
        let mut visited = HashSet::new();
        let mut current_path = Vec::new();

        if edge.tvl >= 10.0 {
            // Only start from pools with sufficient TVL
            dfs_find_cycles(
                edge,
                &edges,
                &mut visited,
                &mut current_path,
                &mut all_cycles,
                0,
                if edge.token_a == CHEESE_MINT {
                    WALLET_CHEESE_BALANCE / 10.0
                } else {
                    edge.reserves_a / 10.0
                },
                cheese_usdc_price,
                jup_prices,
            );
        }
    }

    if all_cycles.is_empty() {
        println!("No profitable cycles found");
        return;
    }

    // Sort cycles by USDC profit
    all_cycles.sort_by(|a, b| {
        let profit_a = a.final_usdc_value - a.initial_usdc_value - a.fees_usdc_value;
        let profit_b = b.final_usdc_value - b.initial_usdc_value - b.fees_usdc_value;
        profit_b
            .partial_cmp(&profit_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for cycle in all_cycles {
        let profit_cheese = cycle.final_cheese - cycle.initial_cheese;
        let profit_usdc = cycle.final_usdc_value - cycle.initial_usdc_value - cycle.fees_usdc_value;

        if profit_usdc <= 1.0 {
            // Skip opportunities less than $1
            continue;
        }

        // Get the starting token info
        let first_step = &cycle.steps[0];
        let start_token = token_symbols
            .get(&first_step.sell_token)
            .cloned()
            .unwrap_or_else(|| {
                first_step
                    .sell_token
                    .chars()
                    .rev()
                    .take(4)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect()
            });

        println!("\n🔄 Arbitrage Cycle (Profit: ${:.2})", profit_usdc);

        // Show starting amount in original token and USD
        println!(
            "├─ Start with: {:.4} {} (${:.2})",
            first_step.amount_in, start_token, cycle.initial_usdc_value
        );

        for (i, step) in cycle.steps.iter().enumerate() {
            let token_a = token_symbols
                .get(&step.sell_token)
                .cloned()
                .unwrap_or_else(|| {
                    step.sell_token
                        .chars()
                        .rev()
                        .take(4)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect()
                });

            let token_b = token_symbols
                .get(&step.buy_token)
                .cloned()
                .unwrap_or_else(|| {
                    step.buy_token
                        .chars()
                        .rev()
                        .take(4)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect()
                });

            // Calculate USD values for this step
            let amount_in_usd = if step.sell_token == CHEESE_MINT {
                step.amount_in * cheese_usdc_price
            } else {
                step.amount_in
                    * jup_prices
                        .get(&step.sell_token)
                        .or_else(|| derived_prices.get(&step.sell_token))
                        .unwrap_or(&0.0)
            };

            let amount_out_usd = if step.buy_token == CHEESE_MINT {
                step.expected_out * cheese_usdc_price
            } else {
                step.expected_out
                    * jup_prices
                        .get(&step.buy_token)
                        .or_else(|| derived_prices.get(&step.buy_token))
                        .unwrap_or(&0.0)
            };

            println!("├─ Step {}: {} → {}", i + 1, token_a, token_b);
            println!("│  ├─ Pool: {} ({})", step.pool_address, step.source);
            println!(
                "│  ├─ Amount In:  {:.4} {} (${:.2})",
                step.amount_in, token_a, amount_in_usd
            );
            println!(
                "│  ├─ Amount Out: {:.4} {} (${:.2})",
                step.expected_out, token_b, amount_out_usd
            );
            println!("│  └─ Fee: {:.2}%", step.fee_percent * 100.0);
        }

        println!(
            "├─ End with: {:.4} 🧀 (${:.2})",
            cycle.final_cheese, cycle.final_usdc_value
        );
        println!(
            "├─ Transaction Cost: {:.6} SOL = ${:.2}",
            cycle.total_fees_sol, cycle.fees_usdc_value
        );
        println!(
            "├─ Total Pool Fees: {:.2}%",
            cycle.pool_fees_paid.iter().sum::<f64>() * 100.0
        );
        println!(
            "└─ Net Profit: ${:.2} ({:.2}%)",
            profit_usdc,
            (profit_usdc / cycle.initial_usdc_value) * 100.0
        );
    }
}

fn find_arbitrage_cycles(
    edges: &[PoolEdge],
    cheese_usdc_price: f64,
    jup_prices: &HashMap<String, f64>,
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
                WALLET_CHEESE_BALANCE / 10.0, // Start with 10% of our CHEESE
                cheese_usdc_price,
                jup_prices,
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
    jup_prices: &HashMap<String, f64>,
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
        let total_fees_sol = path.len() as f64 * SOL_PER_TX;
        let pool_fees: Vec<f64> = path.iter().map(|step| step.fee_percent).collect();

        // Calculate USDC values
        let initial_usdc_value = if path[0].sell_token == CHEESE_MINT {
            path[0].amount_in * cheese_usdc_price
        } else {
            path[0].amount_in * jup_prices.get(&path[0].sell_token).unwrap_or(&0.0)
        };

        let final_usdc_value = expected_out * cheese_usdc_price;

        // Calculate fees in USDC
        let sol_price = jup_prices
            .get("So11111111111111111111111111111111111111112")
            .unwrap_or(&0.0);
        let fees_usdc_value = total_fees_sol * sol_price;

        // Calculate pool fees in USDC
        let pool_fees_usdc: f64 = path
            .iter()
            .enumerate()
            .map(|(i, step)| {
                let amount_value = if step.sell_token == CHEESE_MINT {
                    step.amount_in * cheese_usdc_price
                } else {
                    step.amount_in * jup_prices.get(&step.sell_token).unwrap_or(&0.0)
                };
                amount_value * step.fee_percent
            })
            .sum();

        let total_fees_usdc = fees_usdc_value + pool_fees_usdc;

        // Only add cycle if profit is significant (more than fees)
        let profit_usdc = final_usdc_value - initial_usdc_value - total_fees_usdc;
        if profit_usdc > 1.0 {
            // Only show opportunities > $1
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
                    jup_prices,
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
        "| {:<8} | {:<44} | {:<10} | {:>10} | {:>10} | {:<10} | {:>12} | {:>12} | {:>5} | {:>11} | {:<44} |",
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
