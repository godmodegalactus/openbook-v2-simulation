use anyhow::{Context, bail};
use cli::Args;
use clap::Parser;
use config::Config;

mod cli;
mod market_maker;
mod config;

#[tokio::main()]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    
    let config = std::fs::read_to_string(&args.simulation_configuration).context("Should have been able to read the file")?;
    let config: Config = serde_json::from_str(&config).context("Config file not valid")?;

    if config.users.is_empty() {
        log::error!("Config file is missing payers");
        bail!("No payers");
    }

    if config.markets.is_empty() {
        log::error!("Config file is missing markets");
        bail!("No markets")
    }

    Ok(())
}
