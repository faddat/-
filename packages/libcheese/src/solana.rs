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
use spl_associated_token_account::get_associated_token_address;
use spl_token;
use std::{str::FromStr, time::Duration};
use tokio::time::sleep;

use crate::meteora::{self, MeteoraPool};

const MAX_RETRIES: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_secs(1);

pub struct TradeExecutor {
    pub rpc_client: RpcClient,
    pub wallet: Keypair,
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

    /// Execute a trade on Meteora
    pub async fn execute_trade(
        &self,
        pool: &MeteoraPool,
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
                .execute_trade_internal(pool, input_mint, output_mint, amount_in)
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
        pool: &MeteoraPool,
        input_mint: &str,
        output_mint: &str,
        amount_in: u64,
    ) -> Result<Signature> {
        // 1. Get quote from Meteora for minimum out amount calculation
        println!("Getting quote from Meteora...");
        let quote = meteora::get_meteora_quote(
            &self.http_client,
            &pool.pool_address,
            input_mint,
            output_mint,
            amount_in,
        )
        .await?;

        let minimum_out_amount = (quote.out_amount.parse::<f64>()? * 0.99) as u64; // 1% slippage
        println!(
            "Quote received: in_amount={}, minimum_out_amount={}",
            amount_in, minimum_out_amount
        );

        // 2. Build transaction directly
        println!("Building swap transaction...");
        let input_mint_pubkey = Pubkey::from_str(input_mint)?;
        let mut tx = meteora::build_meteora_swap_transaction(
            pool,
            &self.wallet.pubkey(),
            &input_mint_pubkey,
            amount_in,
            minimum_out_amount,
        )
        .await?;

        // 3. Get and set recent blockhash
        println!("Getting recent blockhash...");
        let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
        tx.message.recent_blockhash = recent_blockhash;

        // 4. Sign transaction
        println!("Signing transaction...");
        tx.sign(&[&self.wallet], tx.message.recent_blockhash);

        // 5. Simulate transaction
        println!("Simulating transaction...");
        match self.simulate_transaction(&tx).await {
            Ok(_) => println!("Transaction simulation successful"),
            Err(e) => {
                println!("Transaction simulation failed: {}", e);
                return Err(e);
            }
        }

        // 6. Send and confirm transaction
        println!("Sending transaction...");
        self.send_and_confirm_transaction(&tx).await
    }

    /// Check if the wallet has sufficient balance for the trade
    async fn check_token_balance(&self, mint: &str, amount: u64) -> Result<()> {
        let token_account = self.find_token_account(mint)?;

        // Check if token account exists
        match self.rpc_client.get_account(&token_account) {
            Ok(_) => (),
            Err(_) => {
                println!(
                    "Token account {} does not exist, creating...",
                    token_account
                );
                self.create_token_account(mint).await?;
            }
        }

        let balance = self.rpc_client.get_token_account_balance(&token_account)?;
        println!(
            "Current balance of {}: {} (need {})",
            mint,
            balance.ui_amount.unwrap_or(0.0),
            amount as f64 / 10f64.powi(balance.decimals as i32)
        );

        // Compare raw amounts (lamports) instead of UI amounts
        if balance.amount.parse::<u64>().unwrap_or(0) < amount {
            return Err(anyhow!(
                "Insufficient balance: have {} {}, need {}",
                balance.ui_amount.unwrap_or(0.0),
                mint,
                amount as f64 / 10f64.powi(balance.decimals as i32)
            ));
        }

        Ok(())
    }

    /// Create token account if it doesn't exist
    async fn create_token_account(&self, mint: &str) -> Result<()> {
        let mint_pubkey = Pubkey::from_str(mint)?;
        let owner = self.wallet.pubkey();

        let create_ix = spl_associated_token_account::instruction::create_associated_token_account(
            &owner,
            &owner,
            &mint_pubkey,
            &spl_token::id(),
        );

        let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
        let tx = Transaction::new_signed_with_payer(
            &[create_ix],
            Some(&owner),
            &[&self.wallet],
            recent_blockhash,
        );

        self.send_and_confirm_transaction(&tx).await?;
        println!("Created token account for mint {}", mint);
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

    /// Simulate a transaction before sending
    async fn simulate_transaction(&self, transaction: &Transaction) -> Result<()> {
        let sim_result = self.rpc_client.simulate_transaction(transaction)?;

        if let Some(err) = sim_result.value.err {
            println!("Simulation error: {:?}", err);
            println!("Transaction details:");
            println!("  Signatures: {:?}", transaction.signatures);
            println!("  Message: {:?}", transaction.message);

            if let Some(logs) = sim_result.value.logs {
                println!("\nTransaction logs:");
                for log in logs {
                    println!("  {}", log);
                }
            }

            if let Some(accounts) = sim_result.value.accounts {
                println!("\nAccount information:");
                for (i, account) in accounts.iter().enumerate() {
                    if let Some(acc) = account {
                        println!("  Account {}: {} lamports={}", i, acc.owner, acc.lamports);
                    }
                }
            }

            return Err(anyhow!("Transaction simulation failed: {:?}", err));
        }
        Ok(())
    }

    /// Send and confirm a transaction
    async fn send_and_confirm_transaction(&self, transaction: &Transaction) -> Result<Signature> {
        let signature = transaction.signatures[0];
        println!("Sending transaction with signature: {}", signature);

        match self.rpc_client.send_and_confirm_transaction(transaction) {
            Ok(_) => {
                println!("Transaction confirmed successfully");
                Ok(signature)
            }
            Err(e) => {
                println!("Transaction failed: {}", e);
                // Try to get more details about the error
                if let Ok(status) = self.rpc_client.get_signature_status(&signature) {
                    println!("Transaction status: {:?}", status);
                }
                Err(anyhow!("Failed to send transaction: {}", e))
            }
        }
    }

    /// Ensure a token account exists for the given mint
    pub async fn ensure_token_account(&self, mint: &str) -> Result<()> {
        let token_account = self.find_token_account(mint)?;

        // Check if token account exists
        match self.rpc_client.get_account(&token_account) {
            Ok(_) => {
                println!("Token account {} exists", token_account);
                Ok(())
            }
            Err(_) => {
                println!("Creating token account for mint {}", mint);
                self.create_token_account(mint).await
            }
        }
    }

    pub async fn get_token_balance(&self, mint: &Pubkey) -> Result<u64> {
        let token_account = get_associated_token_address(&self.wallet.pubkey(), mint);
        match self.rpc_client.get_token_account_balance(&token_account) {
            Ok(balance) => Ok(balance.amount.parse().unwrap_or(0)),
            Err(_) => Ok(0), // Return 0 if account doesn't exist
        }
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
