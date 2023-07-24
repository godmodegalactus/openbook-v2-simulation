use std::{
    collections::HashMap,
    sync::Mutex,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};

use crate::states::TransactionConfirmRecord;
use itertools::Itertools;
use tokio::{sync::RwLock, task::JoinHandle};

// Non atomic version of counters
#[derive(Clone, Default, Debug)]
struct NACounters {
    num_confirmed_txs: u64,
    num_error_txs: u64,
    num_timeout_txs: u64,
    num_successful: u64,
    num_sent: u64,

    // sent transasctions
    num_users_txs: u64,
    num_consume_events_txs: u64,

    // successful transasctions
    succ_users_txs: u64,
    succ_consume_events_txs: u64,

    // errors section
    errors: HashMap<String, u64>,
}

impl NACounters {
    pub fn diff(&self, other: &NACounters) -> NACounters {
        let mut new_error_count = HashMap::new();
        for (error, count) in &self.errors {
            if let Some(v) = other.errors.get(error) {
                new_error_count.insert(error.clone(), *count - *v);
            } else {
                new_error_count.insert(error.clone(), *count);
            }
        }
        NACounters {
            num_confirmed_txs: self.num_confirmed_txs - other.num_confirmed_txs,
            num_error_txs: self.num_error_txs - other.num_error_txs,
            num_timeout_txs: self.num_timeout_txs - other.num_timeout_txs,
            num_successful: self.num_successful - other.num_successful,
            num_sent: self.num_sent - other.num_sent,
            num_users_txs: self.num_users_txs - other.num_users_txs,
            num_consume_events_txs: self.num_consume_events_txs - other.num_consume_events_txs,
            succ_users_txs: self.succ_users_txs - other.succ_users_txs,
            succ_consume_events_txs: self.succ_consume_events_txs - other.succ_consume_events_txs,
            errors: new_error_count,
        }
    }
}

#[derive(Default, Clone, Debug)]
struct Counters {
    num_confirmed_txs: Arc<AtomicU64>,
    num_error_txs: Arc<AtomicU64>,
    num_timeout_txs: Arc<AtomicU64>,
    num_successful: Arc<AtomicU64>,
    num_sent: Arc<AtomicU64>,

    // sent transasctions
    num_users_txs: Arc<AtomicU64>,
    num_consume_events_txs: Arc<AtomicU64>,

    // successful transasctions
    succ_users_txs: Arc<AtomicU64>,
    succ_consume_events_txs: Arc<AtomicU64>,

    // Errors
    errors: Arc<RwLock<HashMap<String, u64>>>,
}

impl Counters {
    pub async fn to_na_counters(&self) -> NACounters {
        NACounters {
            num_confirmed_txs: self.num_confirmed_txs.load(Ordering::Relaxed),
            num_error_txs: self.num_error_txs.load(Ordering::Relaxed),
            num_timeout_txs: self.num_timeout_txs.load(Ordering::Relaxed),
            num_successful: self.num_successful.load(Ordering::Relaxed),
            num_sent: self.num_sent.load(Ordering::Relaxed),

            // sent transasctions
            num_users_txs: self.num_users_txs.load(Ordering::Relaxed),
            num_consume_events_txs: self.num_consume_events_txs.load(Ordering::Relaxed),

            // successful transasctions
            succ_users_txs: self.succ_users_txs.load(Ordering::Relaxed),
            succ_consume_events_txs: self.succ_consume_events_txs.load(Ordering::Relaxed),
            errors: self.errors.read().await.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OpenbookV2SimulationStats {
    recv_limit: usize,
    counters: Counters,
    previous_counters: Arc<Mutex<NACounters>>,
    instant: Instant,
}

impl OpenbookV2SimulationStats {
    pub fn new(nb_users: usize, quotes_per_second: usize, duration_in_sec: usize) -> Self {
        Self {
            recv_limit: nb_users * quotes_per_second * duration_in_sec,
            counters: Counters::default(),
            instant: Instant::now(),
            previous_counters: Arc::new(Mutex::new(NACounters::default())),
        }
    }

    pub fn update_from_tx_status_stream(
        &self,
        tx_confirm_record_reciever: tokio::sync::broadcast::Receiver<TransactionConfirmRecord>,
    ) -> JoinHandle<()> {
        let counters = self.counters.clone();
        let regex = regex::Regex::new(r"Error processing Instruction \d+: ").unwrap();
        tokio::spawn(async move {
            let mut tx_confirm_record_reciever = tx_confirm_record_reciever;
            loop {
                if let Ok(tx_data) = tx_confirm_record_reciever.recv().await {
                    if let Some(_) = tx_data.confirmed_at {
                        counters.num_confirmed_txs.fetch_add(1, Ordering::Relaxed);
                        if let Some(error) = tx_data.error {
                            let error = regex.replace_all(&error, "").to_string();
                            counters.num_error_txs.fetch_add(1, Ordering::Relaxed);
                            let mut lock = counters.errors.write().await;
                            if let Some(value) = lock.get_mut(&error) {
                                *value += 1;
                            } else {
                                lock.insert(error, 1);
                            }
                        } else {
                            counters.num_successful.fetch_add(1, Ordering::Relaxed);

                            if tx_data.is_consume_event {
                                counters
                                    .succ_consume_events_txs
                                    .fetch_add(1, Ordering::Relaxed);
                            } else {
                                counters.succ_users_txs.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    } else {
                        counters.num_timeout_txs.fetch_add(1, Ordering::Relaxed);
                    }
                } else {
                    break;
                }
            }
        })
    }

    pub fn inc_send(&self, is_consume_event: bool) {
        self.counters.num_sent.fetch_add(1, Ordering::Relaxed);

        if is_consume_event {
            self.counters
                .num_consume_events_txs
                .fetch_add(1, Ordering::Relaxed);
        } else {
            self.counters.num_users_txs.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub async fn report(&mut self, is_final: bool, name: &'static str) {
        let time_diff = std::time::Instant::now() - self.instant;
        let mut counters = self.counters.to_na_counters().await;

        println!("\n\n{} at {} secs", name, time_diff.as_secs());
        if !is_final {
            println!("Recently sent transactions could not yet be confirmed and would be confirmed shortly.\n
            diff is wrt previous report");
        } else {
            counters.num_timeout_txs = counters.num_sent - counters.num_confirmed_txs;
        }

        let diff = {
            let mut prev_counter_lock = self.previous_counters.lock().unwrap();
            let diff = counters.diff(&prev_counter_lock);
            *prev_counter_lock = counters.clone();
            diff
        };

        println!(
            "Number of expected marker making transactions : {}",
            self.recv_limit
        );
        println!(
            "Number of transactions Sent:({}) (including keeper) (Diff:({}))",
            counters.num_sent, diff.num_sent,
        );

        println!(
            "User transactions : Sent({}), Successful({}) (Diff : Sent({}), Successful({}))",
            counters.num_users_txs,
            counters.succ_users_txs,
            diff.num_users_txs,
            diff.succ_users_txs,
        );

        println!(
            "Keeper Cosume Events : Sent({}), Successful({}) (Diff : Sent({}), Successful({}))",
            counters.num_consume_events_txs,
            counters.succ_consume_events_txs,
            diff.num_consume_events_txs,
            diff.succ_consume_events_txs
        );

        println!(
            "Transactions confirmed : {}%",
            (counters.num_confirmed_txs * 100)
                .checked_div(counters.num_sent)
                .unwrap_or(0)
        );
        println!(
            "Transactions successful : {}%",
            (counters.num_successful * 100)
                .checked_div(counters.num_sent)
                .unwrap_or(0)
        );
        println!(
            "Transactions timed out : {}%",
            (counters.num_timeout_txs * 100)
                .checked_div(counters.num_sent)
                .unwrap_or(0)
        );
        let top_5_errors = counters
            .errors
            .iter()
            .sorted_by(|x, y| (*y.1).cmp(x.1))
            .take(5)
            .enumerate()
            .collect_vec();
        let mut errors_to_print: String = String::new();
        for (idx, (error, count)) in top_5_errors {
            println!("Error #{idx} : {error} ({count})");
            errors_to_print += format!("{error}({count}),").as_str();
        }
        println!("\n");
    }
}
