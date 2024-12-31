use serde::de::{self, Deserializer};
use serde::Deserialize;

pub const CHEESE_MINT: &str = "A3hzGcTxZNSc7744CWB2LR5Tt9VTtEaQYpP6nwripump";
pub const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

pub fn de_string_to_f64<'de, D>(deserializer: D) -> std::result::Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    s.parse::<f64>().map_err(de::Error::custom)
}

// -----------------------------------
// Helper Functions
// -----------------------------------
pub fn parse_other_token_name(pool_name: &str) -> String {
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
