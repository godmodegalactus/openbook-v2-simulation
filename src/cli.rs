use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    #[arg(short, long, default_value_t = String::from("http://127.0.0.1:8899"))]
    pub rpc_url: String,

    #[arg(short, long, default_value_t = String::from("ws://127.0.0.1:8900"))]
    pub ws_url: String,

    /// tpu fanout
    #[arg(short = 'f', long, default_value_t = 16)]
    pub fanout_size: u64,

    #[arg(short = 'k', long, default_value_t = String::new())]
    pub identity: String,

    #[arg(long, default_value_t = 60)]
    pub duration_in_seconds: u64,

    #[arg(long, default_value_t = 1)]
    pub quotes_per_seconds: u64,

    #[arg(short = 'c', long)]
    pub simulation_configuration: String,

    #[arg(short = 't', long, default_value_t = String::new())]
    pub transaction_save_file: String,

    #[arg(short = 'b', long, default_value_t = String::new())]
    pub block_data_save_file: String,

    #[arg(short = 'a', long, default_value_t = String::new())]
    pub keeper_authority: String,

    #[arg(long, default_value_t = 10)]
    pub transaction_retry_in_ms: u64,
}
