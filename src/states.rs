use serde::Serialize;
use solana_program::{slot_history::Slot, pubkey::Pubkey};
use solana_sdk::signature::Signature;
use chrono::{DateTime, Utc};

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