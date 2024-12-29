//! wallet.rs
//! Example using the `hd-wallet` crate to derive keys for multiple blockchains.

use anyhow::{anyhow, Context, Result};
use bip39::{ErrorKind, Language, Mnemonic, Seed};
use generic_ec::Curve;
use hd_wallet::{DerivationPath, ExtendedKey};
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;

/// Our collection of derived keys for multiple chains.
#[derive(Debug)]
pub struct MultiChainKeys {
    pub eth_key: ExtendedKey<bip39::Secp256k1>, // secp256k1 (Ethereum)
    pub sol_key: ExtendedKey<bip39::Ed25519>,   // ed25519 (Solana)
    pub aptos_key: ExtendedKey<bip39::Ed25519>, // ed25519 (Aptos)
    pub sui_key: ExtendedKey<bip39::Ed25519>,   // ed25519 (Sui)
}

/// Create keys from local `seedphrase` file, deriving each chain's path.
pub fn create_multichain_keys() -> Result<MultiChainKeys> {
    // 1) Read the BIP39 seed phrase from disk
    let seed_phrase = read_seed_phrase_file()?;

    // 2) Parse BIP39 mnemonic
    let mnemonic = Mnemonic::parse_in(Language::English, &seed_phrase)
        .map_err(|e| anyhow!("Error parsing mnemonic: {}", e))?;

    // 3) Convert mnemonic => 64-byte master seed
    let seed = Seed::new(&mnemonic, "");

    // ==================================================
    //  Ethereum path: m/44'/60'/0'/0/0  (secp256k1)
    // ==================================================
    let eth_path = DerivationPath::from_str("m/44'/60'/0'/0/0")?;
    let eth_key = bip39::Secp256k1::derive_from_path(seed.as_bytes(), &Some(eth_path))?;

    // ==================================================
    //  Solana path: m/44'/501'/0'/0'  (ed25519)
    // ==================================================
    let sol_path = DerivationPath::from_str("m/44'/501'/0'/0'")?;
    let sol_key = bip39::Ed25519::derive_from_path(seed.as_bytes(), &Some(sol_path))?;

    // ==================================================
    //  Aptos path: m/44'/637'/0'/0'/0'  (ed25519)
    // ==================================================
    let aptos_path = DerivationPath::from_str("m/44'/637'/0'/0'/0'")?;
    let aptos_key = bip39::Ed25519::derive_from_path(seed.as_bytes(), &Some(aptos_path))?;

    // ==================================================
    //  Sui path: m/44'/784'/0'/0'/0'  (ed25519)
    // ==================================================
    let sui_path = DerivationPath::from_str("m/44'/784'/0'/0'/0'")?;
    let sui_key = bip39::Ed25519::derive_from_path(seed.as_bytes(), &Some(sui_path))?;

    Ok(MultiChainKeys {
        eth_key,
        sol_key,
        aptos_key,
        sui_key,
    })
}

/// Helper: read the `seedphrase` file and trim it.
fn read_seed_phrase_file() -> Result<String> {
    let path = PathBuf::from("seedphrase");
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read seed phrase from {:?}", path))?;
    Ok(raw.trim().to_string())
}

fn main() -> Result<()> {
    let keys = create_multichain_keys()?;

    // Ethereum (Secp256k1)
    println!("Ethereum Key (Extended): {:?}", keys.eth_key);

    // Solana (Ed25519)
    println!("Solana Key (Extended): {:?}", keys.sol_key);

    // Aptos (Ed25519)
    println!("Aptos Key (Extended): {:?}", keys.aptos_key);

    // Sui (Ed25519)
    println!("Sui Key (Extended): {:?}", keys.sui_key);

    Ok(())
}
