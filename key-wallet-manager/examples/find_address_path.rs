//! Prove which derivation path produces given addresses for a mnemonic.
//!
//! Usage: cargo run -p key-wallet-manager --example find_address_path -- \
//!            <mnemonic-file> <max-index> <addr1> [addr2 ...]

use key_wallet::managed_account::address_pool::{AddressPool, AddressPoolType, KeySource};
use key_wallet::wallet::initialization::WalletAccountCreationOptions;
use key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;
use key_wallet::Network;
use key_wallet_manager::WalletManager;
use std::collections::HashSet;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mnemonic = std::fs::read_to_string(&args[1]).expect("read mnemonic file");
    let max_index: u32 = args[2].parse().expect("max index");
    let targets: HashSet<String> = args[3..].iter().cloned().collect();

    let mut wm = WalletManager::<ManagedWalletInfo>::new(Network::Testnet);
    let wallet_id = wm
        .create_wallet_from_mnemonic(
            mnemonic.trim(),
            0,
            WalletAccountCreationOptions::default(),
        )
        .expect("create wallet");

    let info = wm.get_wallet_info(&wallet_id).expect("info");
    let signing = wm.get_wallet(&wallet_id).expect("wallet");

    // Collect (label, xpub, base_path, pool_type) for every funding pool.
    let mut pools = Vec::new();
    for (idx, account) in &signing.accounts.standard_bip44_accounts {
        pools.push((format!("BIP44 account {idx}"), account.account_xpub));
    }
    for (idx, account) in &signing.accounts.coinjoin_accounts {
        pools.push((format!("CoinJoin account {idx}"), account.account_xpub));
    }

    use key_wallet::account::ManagedAccountTrait;
    let mut live_pools = Vec::new();
    for account in info.accounts.all_accounts() {
        for pool in account.managed_account_type().address_pools() {
            live_pools.push((
                format!("{:?}", account.managed_account_type().to_account_type()),
                format!("{:?}", pool.pool_type),
                pool.base_path.clone(),
            ));
        }
    }

    for (acct_label, xpub) in &pools {
        for (label, ptype, base_path) in &live_pools {
            if !label_matches(acct_label, label) {
                continue;
            }
            let pool_type = if ptype == "External" {
                AddressPoolType::External
            } else if ptype == "Internal" {
                AddressPoolType::Internal
            } else {
                continue;
            };
            let mut pool = match AddressPool::new(
                base_path.clone(),
                pool_type,
                max_index,
                Network::Testnet,
                &KeySource::Public(*xpub),
            ) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let ks = KeySource::Public(*xpub);
            if pool.highest_generated.map(|h| h + 1).unwrap_or(0) < max_index {
                let missing = max_index - pool.highest_generated.map(|h| h + 1).unwrap_or(0);
                let _ = pool.generate_addresses(missing, &ks, true);
            }
            for i in 0..max_index {
                if let Some(addr) = pool.address_at_index(i) {
                    let s = addr.to_string();
                    if targets.contains(&s) {
                        println!("FOUND {s} = {acct_label} / {ptype} pool, path {base_path}/{i}");
                    }
                }
            }
        }
    }
    println!("search complete");
}

fn label_matches(acct: &str, managed: &str) -> bool {
    (acct.starts_with("BIP44") && managed.contains("Standard"))
        || (acct.starts_with("CoinJoin") && managed.contains("CoinJoin"))
}
