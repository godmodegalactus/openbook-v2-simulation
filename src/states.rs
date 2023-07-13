use chrono::{DateTime, Utc};
use serde::Serialize;
use solana_program::{pubkey::Pubkey, slot_history::Slot};
use solana_sdk::signature::Signature;

#[derive(Clone, Serialize)]
pub struct TransactionSendRecord {
    pub signature: Signature,
    pub sent_at: DateTime<Utc>,
    pub sent_slot: Slot,
    pub user: Option<Pubkey>,
    pub market: Option<Pubkey>,
    pub is_consume_event: bool,
    pub priority_fees: u64,
}

#[derive(Clone, Serialize)]
pub struct TransactionConfirmRecord {
    pub signature: String,
    pub sent_slot: Slot,
    pub sent_at: String,
    pub confirmed_slot: Option<Slot>,
    pub confirmed_at: Option<String>,
    pub successful: bool,
    pub slot_leader: Option<String>,
    pub error: Option<String>,
    pub user: Option<String>,
    pub market: Option<String>,
    pub block_hash: Option<String>,
    pub slot_processed: Option<Slot>,
    pub is_consume_event: bool,
    pub timed_out: bool,
    pub priority_fees: u64,
}

#[derive(Clone, Serialize)]
pub struct BlockData {
    pub block_hash: String,
    pub block_slot: Slot,
    pub block_leader: String,
    pub total_transactions: u64,
    pub number_of_mm_transactions: u64,
    pub block_time: u64,
    pub cu_consumed: u64,
    pub percentage_filled_by_openbook: f32,
}
