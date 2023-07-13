use anyhow::{bail, Context};
use clap::Parser;
use cli::Args;
use helpers::start_blockhash_polling_service;
use json_config::Config;
use market_maker::start_market_makers;
use openbook_config::Obv2Config;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::commitment_config::CommitmentConfig;
use std::sync::{atomic::AtomicU64, Arc};
use tokio::sync::RwLock;

mod cli;
mod helpers;
mod json_config;
mod market_maker;
mod openbook_config;
mod tpu_manager;

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

    let obv2_config = Obv2Config::try_from(&config).expect("should be convertible to obv2 config");
    let program_id = obv2_config
        .programs
        .iter()
        .find(|x| x.name == "openbook_v2")
        .expect("msg")
        .program_id;
    let (tx_sx, tx_rx) = tokio::sync::mpsc::unbounded_channel();

    // create a task that updates blockhash after every interval
    let rpc_client = Arc::new(RpcClient::new_with_commitment(
        args.rpc_url.to_string(),
        CommitmentConfig::finalized(),
    ));

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .await
        .expect("Rpc URL is not working");
    let last_slot = rpc_client.get_slot().await.expect("Rpc URL is not working");
    let blockhash_rw = Arc::new(RwLock::new(recent_blockhash));
    let current_slot = Arc::new(AtomicU64::new(last_slot));
    let bh_polling_task =
        start_blockhash_polling_service(blockhash_rw.clone(), current_slot, rpc_client);

    // start tpu manager

    // start market making
    let market_makers_task =
        start_market_makers(&args, &obv2_config, &program_id, tx_sx, blockhash_rw);

    Ok(())
}
