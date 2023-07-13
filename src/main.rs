use anyhow::{bail, Context};
use clap::Parser;
use cli::Args;
use confirmation_strategy::confirmations_by_blocks;
use helpers::{create_transaction_bridge, start_blockhash_polling_service};
use json_config::Config;
use market_maker::start_market_makers;
use openbook_config::Obv2Config;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{commitment_config::CommitmentConfig, signature::Keypair};
use stats::OpenbookV2SimulationStats;
use std::{
    sync::{atomic::AtomicU64, Arc},
    time::Duration,
};
use tokio::sync::{mpsc::unbounded_channel, RwLock};

mod cli;
mod confirmation_strategy;
mod helpers;
mod json_config;
mod market_maker;
mod openbook_config;
mod states;
mod tpu_manager;
mod stats;

#[tokio::main(flavor = "multi_thread", worker_threads = 16)]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let config = std::fs::read_to_string(&args.simulation_configuration)
        .context("Should have been able to read the file")?;
    let config: Config = serde_json::from_str(&config).context("Config file not valid")?;

    if config.users.is_empty() {
        log::error!("Config file is missing payers");
        bail!("No payers");
    }

    if config.markets.is_empty() {
        log::error!("Config file is missing markets");
        bail!("No markets")
    }

    let identity = if !args.identity.is_empty() {
        let identity_file = tokio::fs::read_to_string(args.identity.as_str())
            .await
            .expect("Cannot find the identity file provided");
        let identity_bytes: Vec<u8> =
            serde_json::from_str(&identity_file).expect("Keypair file invalid");
        Keypair::from_bytes(identity_bytes.as_slice()).expect("Keypair file invalid")
    } else {
        Keypair::new()
    };

    let obv2_config = Obv2Config::try_from(&config).expect("should be convertible to obv2 config");
    let program_id = obv2_config
        .programs
        .iter()
        .find(|x| x.name == "openbook_v2")
        .expect("msg")
        .program_id;
    let (tx_sx, tx_rx) = unbounded_channel();

    let rpc_client = Arc::new(RpcClient::new_with_commitment(
        args.rpc_url.to_string(),
        CommitmentConfig::finalized(),
    ));

    let openbook_simulation_stats = OpenbookV2SimulationStats::new(
        config.users.len(),
        args.quotes_per_seconds as usize,
        args.duration_in_seconds as usize,
    );

    // create a task that updates blockhash after every interval
    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .await
        .expect("Rpc URL is not working");
    let last_slot = rpc_client.get_slot().await.expect("Rpc URL is not working");
    let blockhash_rw = Arc::new(RwLock::new(recent_blockhash));
    let current_slot = Arc::new(AtomicU64::new(last_slot));
    let bh_polling_task = start_blockhash_polling_service(
        blockhash_rw.clone(),
        current_slot.clone(),
        rpc_client.clone(),
    );

    // start tpu manager
    let (tx_send_record_sx, tx_send_record_rx) = unbounded_channel();
    let tpu_manager = Arc::new(
        tpu_manager::TpuManager::new(
            rpc_client.clone(),
            args.ws_url.clone(),
            16,
            identity,
            tx_send_record_sx,
            openbook_simulation_stats.clone(),
        )
        .await,
    );
    tpu_manager.force_reset_after_every(Duration::from_secs(600)); // reset every 10 minutes

    // start confirmations by blocks
    let (tx_confirmation_sx, tx_confirmation_rx) = tokio::sync::broadcast::channel(8192);
    let (blocks_confirmation_sx, blocks_confirmation_rx) = tokio::sync::broadcast::channel(8192);
    let mut other_services = confirmations_by_blocks(
        rpc_client.clone(),
        tx_send_record_rx,
        tx_confirmation_sx,
        blocks_confirmation_sx,
        current_slot.load(std::sync::atomic::Ordering::Relaxed),
    );
    openbook_simulation_stats.update_from_tx_status_stream(tx_confirmation_rx);

    other_services.push(bh_polling_task);
    // start transaction send bridge
    let transaction_send_bridge_task =
        create_transaction_bridge(tx_rx, tpu_manager, 16, Duration::from_millis(5));
    other_services.push(transaction_send_bridge_task);

    // start market making
    let market_makers_task = start_market_makers(
        &args,
        &obv2_config,
        &program_id,
        tx_sx,
        blockhash_rw,
        current_slot.clone(),
    );

    // task which updates stats
    let mut openbook_simulation_stats = openbook_simulation_stats.clone();
    let reporting_thread = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            openbook_simulation_stats.report(false, "openbook v2 simulation").await;
        }
    });
    other_services.push(reporting_thread);

    match tokio::time::timeout(
        Duration::from_secs(args.duration_in_seconds),
        futures::future::select_all(market_makers_task),
    )
    .await
    {
        Err(_) => {
            // market making tasks should timeout anyways it is normal behavior
        }
        Ok(_) => {
            log::error!("user unexpectedly exited sending transactions");
        }
    }
    let _ = futures::future::select_all(other_services).await;
    Ok(())
}
