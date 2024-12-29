use anyhow::{anyhow, Context, Result};
use bip39::{Language, Mnemonic, Seed};
use ed25519_dalek::{
    Keypair as Ed25519Keypair, PublicKey as EdPublic, SecretKey as EdSecretKey,
    Signature as EdSignature, Signer as EdSigner,
};
use k256::ecdsa::{
    signature::Signer as EcdsaSigner, Signature as EthSignature, SigningKey as K256SigningKey,
};
use std::fs;
use std::path::PathBuf;

/// A container for all chain-specific private keys derived from the seed phrase
#[derive(Debug)]
pub struct MultiChainKeys {
    /// Ethereum private key (secp256k1)
    pub eth_key: K256SigningKey,

    /// Solana keypair (ed25519)
    pub sol_keypair: Ed25519Keypair,

    /// Sui keypair (ed25519-based)
    pub sui_key: Ed25519Keypair,

    /// Aptos keypair (ed25519-based)
    pub aptos_key: Ed25519Keypair,
}

/// Load the local `seedphrase` file, parse as BIP39, and derive keys for each chain.
pub fn create_multichain_keys() -> Result<MultiChainKeys> {
    // 1) read raw seed phrase from file
    let seed_phrase = read_seed_phrase_file()?;

    // 2) parse as BIP39 mnemonic (English)
    let mnemonic = Mnemonic::from_phrase(&seed_phrase, Language::English)
        .map_err(|e| anyhow!("Error parsing mnemonic: {}", e))?;

    // 3) convert mnemonic -> seed (512 bits). Typically you might also pass an optional passphrase.
    let seed = Seed::new(&mnemonic, "");

    // For demonstration, we’ll just create sub-seeds for each chain.
    // In real usage, you’d use each chain’s standard derivation path with slip10, etc.

    let master_seed_bytes = seed.as_bytes(); // 64 bytes

    //
    // Ethereum: use first 32 bytes as secp256k1 secret key
    //
    let eth_signing_key = K256SigningKey::from_bytes(&master_seed_bytes[0..32])
        .map_err(|e| anyhow!("Error creating ETH key: {}", e))?;

    //
    // Solana: use next 32 bytes as the ‘seed’ for ed25519
    //
    let sol_secret = EdSecretKey::from_bytes(&master_seed_bytes[32..64])
        .map_err(|e| anyhow!("Error building Solana secret: {}", e))?;
    let sol_public = EdPublic::from(&sol_secret);
    let sol_keypair = Ed25519Keypair {
        secret: sol_secret,
        public: sol_public,
    };

    //
    // Sui: for demonstration, just reuse the first 32 bytes
    //
    let sui_secret = EdSecretKey::from_bytes(&master_seed_bytes[0..32])
        .map_err(|e| anyhow!("Error building Sui secret: {}", e))?;
    let sui_public = EdPublic::from(&sui_secret);
    let sui_keypair = Ed25519Keypair {
        secret: sui_secret,
        public: sui_public,
    };

    //
    // Aptos: similarly, demonstrate using next 32
    //
    let aptos_secret = EdSecretKey::from_bytes(&master_seed_bytes[32..64])
        .map_err(|e| anyhow!("Error building Aptos secret: {}", e))?;
    let aptos_public = EdPublic::from(&aptos_secret);
    let aptos_keypair = Ed25519Keypair {
        secret: aptos_secret,
        public: aptos_public,
    };

    // Return them all
    Ok(MultiChainKeys {
        eth_key: eth_signing_key,
        sol_keypair,
        sui_key: sui_keypair,
        aptos_key: aptos_keypair,
    })
}

/// Internal helper to read the `seedphrase` file from disk.
fn read_seed_phrase_file() -> Result<String> {
    let path = PathBuf::from("seedphrase");
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read seed phrase from {:?}", path))?;
    Ok(contents.trim().to_string())
}

impl MultiChainKeys {
    /// Signs a transaction for the specified chain.
    ///
    /// # Arguments
    ///
    /// * `chain`: The name of the blockchain ("eth", "sol", "sui", "aptos").
    /// * `message`: The transaction data to sign (as bytes).
    ///
    /// # Returns
    ///
    /// Returns a vector of bytes representing the signature, or an error if the
    /// chain is not supported or signing fails.
    pub fn sign_transaction(&self, chain: &str, message: &[u8]) -> Result<Vec<u8>> {
        match chain {
            "eth" => {
                let signature: EthSignature = self.eth_key.sign(message);
                Ok(signature.to_bytes().to_vec())
            }
            "sol" => {
                let signature: EdSignature = self.sol_keypair.sign(message);
                Ok(signature.to_bytes().to_vec())
            }
            "sui" => {
                let signature: EdSignature = self.sui_key.sign(message);
                Ok(signature.to_bytes().to_vec())
            }
            "aptos" => {
                let signature: EdSignature = self.aptos_key.sign(message);
                Ok(signature.to_bytes().to_vec())
            }
            _ => Err(anyhow!("Unsupported chain: {}", chain)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bip39::{Language, Mnemonic};
    use ed25519_dalek::Keypair as Ed25519Keypair;
    use rand_core::OsRng;

    #[test]
    fn test_sign_eth_transaction() {
        let mnemonic = Mnemonic::from_phrase(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            Language::English,
        )
        .unwrap();
        let seed = Seed::new(&mnemonic, "");
        let master_seed_bytes = seed.as_bytes();
        let eth_signing_key = K256SigningKey::from_bytes(&master_seed_bytes[0..32]).unwrap();

        let keys = MultiChainKeys {
            eth_key: eth_signing_key,
            sol_keypair: Ed25519Keypair::generate(&mut OsRng),
            sui_key: Ed25519Keypair::generate(&mut OsRng),
            aptos_key: Ed25519Keypair::generate(&mut OsRng),
        };

        let message = b"hello world";
        let signature = keys.sign_transaction("eth", message).unwrap();
        assert_eq!(signature.len(), 65); // Ethereum ECDSA signature length
    }

    #[test]
    fn test_sign_sol_transaction() {
        let mnemonic = Mnemonic::from_phrase(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            Language::English,
        )
        .unwrap();
        let seed = Seed::new(&mnemonic, "");
        let master_seed_bytes = seed.as_bytes();

        let sol_secret = ed25519_dalek::SecretKey::from_bytes(&master_seed_bytes[32..64]).unwrap();
        let sol_public = ed25519_dalek::PublicKey::from(&sol_secret);
        let sol_keypair = Ed25519Keypair {
            secret: sol_secret,
            public: sol_public,
        };

        let keys = MultiChainKeys {
            eth_key: K256SigningKey::generate(&mut OsRng),
            sol_keypair,
            sui_key: Ed25519Keypair::generate(&mut OsRng),
            aptos_key: Ed25519Keypair::generate(&mut OsRng),
        };

        let message = b"hello world";
        let signature = keys.sign_transaction("sol", message).unwrap();
        assert_eq!(signature.len(), 64); // Solana/Ed25519 signature length
    }

    #[test]
    fn test_sign_sui_transaction() {
        let mnemonic = Mnemonic::from_phrase(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            Language::English,
        )
        .unwrap();
        let seed = Seed::new(&mnemonic, "");
        let master_seed_bytes = seed.as_bytes();

        let sui_secret = ed25519_dalek::SecretKey::from_bytes(&master_seed_bytes[0..32]).unwrap();
        let sui_public = ed25519_dalek::PublicKey::from(&sui_secret);
        let sui_keypair = Ed25519Keypair {
            secret: sui_secret,
            public: sui_public,
        };

        let keys = MultiChainKeys {
            eth_key: K256SigningKey::generate(&mut OsRng),
            sol_keypair: Ed25519Keypair::generate(&mut OsRng),
            sui_key: sui_keypair,
            aptos_key: Ed25519Keypair::generate(&mut OsRng),
        };

        let message = b"hello world";
        let signature = keys.sign_transaction("sui", message).unwrap();
        assert_eq!(signature.len(), 64); // Sui/Ed25519 signature length
    }

    #[test]
    fn test_sign_aptos_transaction() {
        let mnemonic = Mnemonic::from_phrase(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            Language::English,
        )
        .unwrap();
        let seed = Seed::new(&mnemonic, "");
        let master_seed_bytes = seed.as_bytes();

        let aptos_secret =
            ed25519_dalek::SecretKey::from_bytes(&master_seed_bytes[32..64]).unwrap();
        let aptos_public = ed25519_dalek::PublicKey::from(&aptos_secret);
        let aptos_keypair = Ed25519Keypair {
            secret: aptos_secret,
            public: aptos_public,
        };

        let keys = MultiChainKeys {
            eth_key: K256SigningKey::generate(&mut OsRng),
            sol_keypair: Ed25519Keypair::generate(&mut OsRng),
            sui_key: Ed25519Keypair::generate(&mut OsRng),
            aptos_key: aptos_keypair,
        };

        let message = b"hello world";
        let signature = keys.sign_transaction("aptos", message).unwrap();
        assert_eq!(signature.len(), 64); // Aptos/Ed25519 signature length
    }

    #[test]
    fn test_sign_unsupported_chain() {
        let keys = MultiChainKeys {
            eth_key: K256SigningKey::generate(&mut OsRng),
            sol_keypair: Ed25519Keypair::generate(&mut OsRng),
            sui_key: Ed25519Keypair::generate(&mut OsRng),
            aptos_key: Ed25519Keypair::generate(&mut OsRng),
        };
        let message = b"hello world";
        let result = keys.sign_transaction("bitcoin", message);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Unsupported chain: bitcoin"
        );
    }
}
