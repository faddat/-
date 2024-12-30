use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
    transaction::Transaction,
};
use std::{str::FromStr, time::Duration};
use tokio::time::sleep;

const JUPITER_QUOTE_API: &str = "https://quote-api.jup.ag/v6/quote";
const JUPITER_SWAP_API: &str = "https://quote-api.jup.ag/v6/swap";
const MAX_RETRIES: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Serialize)]
struct JupiterQuoteRequest {
    input_mint: String,
    output_mint: String,
    amount: String,    // Amount in lamports/smallest decimals
    slippage_bps: u64, // e.g. 50 = 0.5%
}

#[derive(Debug, Deserialize, Clone, Serialize)]
struct JupiterQuoteResponse {
    input_mint: String,
    output_mint: String,
    in_amount: String,
    out_amount: String,
    other_amount_threshold: String,
    swap_mode: String,
    slippage_bps: u64,
    platform_fee: Option<PlatformFee>,
    price_impact_pct: String,
    route_plan: Vec<RoutePlanStep>,
    context_slot: u64,
    time_taken: f64,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
struct PlatformFee {
    amount: String,
    fee_bps: u64,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
struct RoutePlanStep {
    swap_info: SwapInfo,
    percent: u64,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
struct SwapInfo {
    ammKey: String,
    label: String,
    input_mint: String,
    output_mint: String,
    in_amount: String,
    out_amount: String,
    fee_amount: String,
    fee_mint: String,
}

#[derive(Debug, Serialize)]
struct JupiterSwapRequest {
    user_public_key: String,
    quote_response: JupiterQuoteResponse,
}

#[derive(Debug, Deserialize)]
struct JupiterSwapResponse {
    swap_transaction: String,
}

pub struct TradeExecutor {
    rpc_client: RpcClient,
    wallet: Keypair,
    http_client: Client,
}

impl TradeExecutor {
    pub fn new(rpc_url: &str, wallet_keypair: Keypair) -> Self {
        let rpc_client =
            RpcClient::new_with_commitment(rpc_url.to_string(), CommitmentConfig::confirmed());
        let http_client = Client::new();
        Self {
            rpc_client,
            wallet: wallet_keypair,
            http_client,
        }
    }

    /// Execute a trade on any pool using Jupiter
    pub async fn execute_trade(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount_in: u64,
        slippage_bps: u64,
    ) -> Result<Signature> {
        // Check balance before trading
        self.check_token_balance(input_mint, amount_in).await?;

        for retry in 0..MAX_RETRIES {
            if retry > 0 {
                println!(
                    "Retrying trade execution (attempt {}/{})",
                    retry + 1,
                    MAX_RETRIES
                );
                sleep(RETRY_DELAY).await;
            }

            match self
                .execute_trade_internal(input_mint, output_mint, amount_in, slippage_bps)
                .await
            {
                Ok(sig) => {
                    println!("Trade executed successfully! Signature: {}", sig);
                    println!("View transaction: https://solscan.io/tx/{}", sig);
                    return Ok(sig);
                }
                Err(e) if retry < MAX_RETRIES - 1 => {
                    println!("Trade execution failed: {}. Retrying...", e);
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        Err(anyhow!("Max retries exceeded"))
    }

    async fn execute_trade_internal(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount_in: u64,
        slippage_bps: u64,
    ) -> Result<Signature> {
        // 1. Get quote from Jupiter
        let quote = self
            .get_jupiter_quote(input_mint, output_mint, amount_in, slippage_bps)
            .await?;

        println!(
            "Got quote: {} -> {} ({} -> {})",
            input_mint, output_mint, quote.in_amount, quote.out_amount
        );

        // 2. Get swap transaction
        let swap_tx = self.get_jupiter_swap_transaction(&quote).await?;

        // 3. Deserialize and sign transaction
        let tx: Transaction = bincode::deserialize(&base64::decode(swap_tx)?)?;

        // 4. Simulate transaction with detailed error reporting
        match self.simulate_transaction(&tx).await {
            Ok(_) => println!("Transaction simulation successful"),
            Err(e) => {
                println!("Transaction simulation failed: {}", e);
                return Err(e);
            }
        }

        // 5. Send and confirm transaction
        self.send_and_confirm_transaction(&tx).await
    }

    /// Check if the wallet has sufficient balance for the trade
    async fn check_token_balance(&self, mint: &str, amount: u64) -> Result<()> {
        let token_account = self.find_token_account(mint)?;
        let balance = self.rpc_client.get_token_account_balance(&token_account)?;

        if balance.ui_amount.unwrap_or(0.0) * 10f64.powi(balance.decimals as i32) < amount as f64 {
            return Err(anyhow!(
                "Insufficient balance: have {} {}, need {}",
                balance.ui_amount.unwrap_or(0.0),
                mint,
                amount
            ));
        }

        Ok(())
    }

    /// Find the associated token account for a given mint
    fn find_token_account(&self, mint: &str) -> Result<Pubkey> {
        let mint_pubkey = Pubkey::from_str(mint)?;
        let owner = self.wallet.pubkey();

        Ok(spl_associated_token_account::get_associated_token_address(
            &owner,
            &mint_pubkey,
        ))
    }

    async fn get_jupiter_quote(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
        slippage_bps: u64,
    ) -> Result<JupiterQuoteResponse> {
        let request = JupiterQuoteRequest {
            input_mint: input_mint.to_string(),
            output_mint: output_mint.to_string(),
            amount: amount.to_string(),
            slippage_bps,
        };

        let response = self
            .http_client
            .get(JUPITER_QUOTE_API)
            .query(&request)
            .send()
            .await?
            .error_for_status()?;

        Ok(response.json().await?)
    }

    async fn get_jupiter_swap_transaction(&self, quote: &JupiterQuoteResponse) -> Result<String> {
        let request = JupiterSwapRequest {
            user_public_key: self.wallet.pubkey().to_string(),
            quote_response: quote.clone(),
        };

        let response = self
            .http_client
            .post(JUPITER_SWAP_API)
            .json(&request)
            .send()
            .await?
            .error_for_status()?;

        let swap_response: JupiterSwapResponse = response.json().await?;
        Ok(swap_response.swap_transaction)
    }

    /// Simulate a transaction before sending
    async fn simulate_transaction(&self, transaction: &Transaction) -> Result<()> {
        self.rpc_client
            .simulate_transaction(transaction)
            .map_err(|e| anyhow!("Transaction simulation failed: {}", e))?;
        Ok(())
    }

    /// Send and confirm a transaction
    async fn send_and_confirm_transaction(&self, transaction: &Transaction) -> Result<Signature> {
        let signature = self
            .rpc_client
            .send_and_confirm_transaction(transaction)
            .map_err(|e| anyhow!("Failed to send transaction: {}", e))?;
        Ok(signature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_trade_executor() {
        // TODO: Add tests
    }
}
