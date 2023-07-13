use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use log::{debug, info};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_program::hash::Hash;
use solana_sdk::transaction::Transaction;
use tokio::{
    sync::{mpsc::UnboundedReceiver, RwLock},
    task::JoinHandle,
    time::Instant,
};

use crate::{states::TransactionSendRecord, tpu_manager::TpuManager};

pub async fn get_new_latest_blockhash(client: Arc<RpcClient>, blockhash: &Hash) -> Option<Hash> {
    let start = Instant::now();
    while start.elapsed().as_secs() < 5 {
        if let Ok(new_blockhash) = client.get_latest_blockhash().await {
            if new_blockhash != *blockhash {
                return Some(new_blockhash);
            }
        }
        debug!("Got same blockhash ({:?}), will retry...", blockhash);

        // Retry ~twice during a slot
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    None
}

pub async fn poll_blockhash_and_slot(
    blockhash: Arc<RwLock<Hash>>,
    slot: &AtomicU64,
    client: Arc<RpcClient>,
) {
    let mut blockhash_last_updated = Instant::now();
    //let mut last_error_log = Instant::now();
    loop {
        let client = client.clone();
        let old_blockhash = *blockhash.read().await;

        match client.get_slot().await {
            Ok(new_slot) => slot.store(new_slot, Ordering::Release),
            Err(e) => {
                info!("Failed to download slot: {}, skip", e);
                continue;
            }
        }

        if let Some(new_blockhash) = get_new_latest_blockhash(client, &old_blockhash).await {
            {
                *blockhash.write().await = new_blockhash;
            }
            blockhash_last_updated = Instant::now();
        } else {
            log::error!("Error updating recent blockhash");
            if blockhash_last_updated.elapsed().as_secs() > 120 {
                log::error!("Failed to update blockhash quitting task");
                break;
            }
        }

        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

pub fn start_blockhash_polling_service(
    blockhash: Arc<RwLock<Hash>>,
    current_slot: Arc<AtomicU64>,
    client: Arc<RpcClient>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        poll_blockhash_and_slot(blockhash.clone(), current_slot.as_ref(), client).await;
    })
}

pub fn create_transaction_bridge(
    tx_rx: UnboundedReceiver<(Transaction, TransactionSendRecord)>,
    tpu_manager: Arc<TpuManager>,
    max_batch_size: usize,
    recv_timeout: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut transactions = vec![];
        transactions.reserve(max_batch_size);
        let mut tx_rx = tx_rx;
        loop {
            if transactions.len() < max_batch_size {
                match tokio::time::timeout(recv_timeout, tx_rx.recv()).await {
                    Ok(Some(tx)) => {
                        transactions.push(tx);
                        continue;
                    }
                    Ok(None) => {
                        // channel broken
                        break;
                    }
                    Err(_) => {
                        // timed out continue to send pending transactions
                    }
                }
            }
            if transactions.is_empty() {
                continue;
            }

            // create async task that sends tranasctions over TPU
            let transactions: Vec<(Transaction, TransactionSendRecord)> =
                transactions.drain(..).collect();
            let tpu_manager = tpu_manager.clone();
            tokio::spawn(async move {
                tpu_manager.send_transaction_batch(&transactions).await;
            });
        }
    })
}
