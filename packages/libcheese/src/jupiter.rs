use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;

/// calls Jupiter v2 price endpoint with `showExtraInfo=true` for the given mints
/// returns a map from mint -> float price (and ignores extra info).
pub async fn fetch_jupiter_prices(
    client: &Client,
    mints: &[String],
) -> Result<HashMap<String, f64>> {
    if mints.is_empty() {
        return Ok(HashMap::new());
    }

    // Build comma-separated IDs
    let joined = mints.join(",");
    let url = format!(
        "https://api.jup.ag/price/v2?ids={}&showExtraInfo=true",
        joined
    );
    println!("Fetching Jupiter v2 prices from: {}", url);

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!(
            "Jupiter v2 price request failed: {}",
            resp.status()
        ));
    }

    let parsed: JupiterV2PriceResponse = resp.json().await?;

    // Now let's create a simpler map: mint -> price
    let mut result_map = HashMap::new();

    for (mint, maybe_item) in parsed.data {
        if let Some(item) = maybe_item {
            // item.price is a string, parse to f64
            if let Ok(val) = item.price.parse::<f64>() {
                result_map.insert(mint, val);
            } else {
                // If parse fails, store 0.0 or skip
                println!(
                    "Warning: Jupiter price for mint {} is not parseable: {:?}",
                    mint, item.price
                );
            }
        } else {
            // the API returned null for this mint
            println!("Jupiter returned null for mint {mint}");
        }
    }

    Ok(result_map)
}

#[derive(Debug, Deserialize)]
struct JupiterV2PriceResponse {
    // The top-level has a `data` object containing many mints
    data: HashMap<String, Option<JupiterV2PriceItem>>,
    #[serde(default)]
    timeTaken: f64, // optional
}

#[derive(Debug, Deserialize)]
struct JupiterV2PriceItem {
    id: String,
    #[serde(default)]
    r#type: String,
    // The "price" is returned as string - we can parse to f64
    price: String,
    #[serde(default)]
    extraInfo: Option<JupiterV2ExtraInfo>,
}

#[derive(Debug, Deserialize)]
struct JupiterV2ExtraInfo {
    #[serde(default)]
    lastSwappedPrice: Option<JupiterV2LastSwapped>,
    #[serde(default)]
    quotedPrice: Option<JupiterV2QuotedPrice>,
    #[serde(default)]
    confidenceLevel: Option<String>,
    // Depth or other fields omitted for brevity
}

#[derive(Debug, Deserialize)]
struct JupiterV2LastSwapped {
    #[serde(default)]
    lastJupiterSellAt: Option<u64>,
    #[serde(default)]
    lastJupiterSellPrice: Option<String>,
    #[serde(default)]
    lastJupiterBuyAt: Option<u64>,
    #[serde(default)]
    lastJupiterBuyPrice: Option<String>,
    // etc.
}

#[derive(Debug, Deserialize)]
struct JupiterV2QuotedPrice {
    #[serde(default)]
    buyPrice: Option<String>,
    #[serde(default)]
    buyAt: Option<u64>,
    #[serde(default)]
    sellPrice: Option<String>,
    #[serde(default)]
    sellAt: Option<u64>,
}
