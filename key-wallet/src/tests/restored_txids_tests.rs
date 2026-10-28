//! Tests for `ManagedWalletInfo::restore_known_txids`: a wallet loaded without
//! its transaction records replays already-scanned blocks during a resumed
//! rescan.

use dashcore::blockdata::transaction::{OutPoint, Transaction};
use dashcore::hashes::Hash;
use dashcore::{BlockHash, TxIn, TxOut, Txid};

use crate::test_utils::TestWalletContext;
use crate::transaction_checking::{BlockInfo, TransactionContext, WalletTransactionChecker};
use crate::wallet::ManagedWalletInfo;
use crate::Network;

const FUNDING: u64 = 100_000_000;
const PAID_OUT: u64 = 3_000_000;
const CHANGE: u64 = 96_999_759;

fn in_block(height: u32) -> TransactionContext {
    TransactionContext::InBlock(BlockInfo::new(
        height,
        BlockHash::from([height as u8; 32]),
        1_700_000_000,
    ))
}

fn tx(inputs: &[OutPoint], outputs: Vec<TxOut>) -> Transaction {
    Transaction {
        version: 2,
        lock_time: 0,
        input: inputs
            .iter()
            .map(|op| TxIn {
                previous_output: *op,
                ..Default::default()
            })
            .collect(),
        output: outputs,
        special_transaction_payload: None,
    }
}

fn pay(address: &dashcore::Address, value: u64) -> TxOut {
    TxOut {
        value,
        script_pubkey: address.script_pubkey(),
    }
}

/// A wallet that received `FUNDING` in block 100 and spent it in block 101,
/// `PAID_OUT` to a foreign address and `CHANGE` back to itself. Returns the
/// context, the spend, and a second, reloaded engine: built the way a host
/// loads a wallet, with its accounts and unspent coins and no records.
async fn reloaded_after_spend() -> (TestWalletContext, Transaction, Transaction, ManagedWalletInfo)
{
    let mut ctx = TestWalletContext::new_random();
    let change_address = ctx
        .managed_wallet
        .first_bip44_managed_account_mut()
        .unwrap()
        .next_change_address(Some(&ctx.xpub), true)
        .unwrap();
    let funding = tx(
        &[OutPoint::new(Txid::from_byte_array([2; 32]), 0)],
        vec![pay(&ctx.receive_address, FUNDING)],
    );
    let spend = tx(
        &[OutPoint::new(funding.txid(), 0)],
        vec![
            pay(&dashcore::Address::dummy(Network::Testnet, 7), PAID_OUT),
            pay(&change_address, CHANGE),
        ],
    );
    for (t, height) in [(&funding, 100), (&spend, 101)] {
        let result = ctx
            .managed_wallet
            .check_core_transaction(t, in_block(height), &mut ctx.wallet, true, true)
            .await;
        assert!(result.is_relevant);
    }

    let mut reloaded = ManagedWalletInfo::from_wallet(&ctx.wallet, 0);
    let unspent: Vec<_> = ctx
        .managed_wallet
        .first_bip44_managed_account_mut()
        .unwrap()
        .utxos
        .values()
        .cloned()
        .collect();
    assert_eq!(unspent.len(), 1, "only the change is unspent");
    let account = reloaded.accounts.standard_bip44_accounts.get_mut(&0).unwrap();
    for utxo in unspent {
        account.utxos.insert(utxo.outpoint, utxo);
    }
    (ctx, funding, spend, reloaded)
}

#[tokio::test]
async fn replay_without_restored_txids_records_a_spend_as_change_alone() {
    let (mut ctx, _, spend, mut reloaded) = reloaded_after_spend().await;

    let replay =
        reloaded.check_core_transaction(&spend, in_block(101), &mut ctx.wallet, true, true).await;

    // The hazard `restore_known_txids` exists for: the funding coin is not in
    // the reloaded `utxos`, so the spend reads as an incoming payment.
    let record = replay.new_records.first().expect("the replay records the spend again");
    assert!(record.input_details.is_empty());
    assert_eq!(record.net_amount, CHANGE as i64);
}

#[tokio::test]
async fn replay_of_a_restored_txid_records_nothing_and_keeps_the_coins() {
    let (mut ctx, funding, spend, mut reloaded) = reloaded_after_spend().await;
    reloaded.restore_known_txids([funding.txid(), spend.txid()]);
    let coins_before = reloaded.accounts.standard_bip44_accounts[&0].utxos.clone();

    for (t, height) in [(&funding, 100), (&spend, 101)] {
        let replay =
            reloaded.check_core_transaction(t, in_block(height), &mut ctx.wallet, true, true).await;
        assert!(replay.is_relevant, "the replayed transaction still matches the wallet");
        assert!(!replay.is_new_transaction);
        assert!(replay.new_records.is_empty(), "a restored txid is not recorded again");
        assert!(replay.updated_records.is_empty());
    }
    assert_eq!(
        reloaded.accounts.standard_bip44_accounts[&0].utxos, coins_before,
        "the reloaded coins are neither credited again nor spent again"
    );
}

#[tokio::test]
async fn a_transaction_not_restored_is_still_recorded_during_the_replay() {
    let (mut ctx, funding, spend, mut reloaded) = reloaded_after_spend().await;
    reloaded.restore_known_txids([funding.txid(), spend.txid()]);

    // A payment the earlier session never saw, found by the rescan.
    let payment = tx(
        &[OutPoint::new(Txid::from_byte_array([3; 32]), 0)],
        vec![pay(&ctx.receive_address, 5_000)],
    );
    let result =
        reloaded.check_core_transaction(&payment, in_block(101), &mut ctx.wallet, true, true).await;

    assert!(result.is_new_transaction);
    assert_eq!(result.new_records.len(), 1);
    assert_eq!(result.new_records[0].net_amount, 5_000);
}

#[tokio::test]
async fn a_restored_txid_seen_unconfirmed_is_not_skipped() {
    let (mut ctx, funding, _, mut reloaded) = reloaded_after_spend().await;
    let pending = tx(
        &[OutPoint::new(Txid::from_byte_array([4; 32]), 0)],
        vec![pay(&ctx.receive_address, 7_000)],
    );
    // A host that wrongly lists an unconfirmed row as mined: the mempool
    // sighting is still recorded, since only block deliveries are skipped.
    reloaded.restore_known_txids([funding.txid(), pending.txid()]);

    let result = reloaded
        .check_core_transaction(&pending, TransactionContext::Mempool, &mut ctx.wallet, true, true)
        .await;

    assert_eq!(result.new_records.len(), 1);
}
