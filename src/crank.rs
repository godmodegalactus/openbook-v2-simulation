use crate::{
    openbook_config::Obv2Config, openbook_v2_sink::OpenbookV2CrankSink,
    states::TransactionSendRecord,
};
use anyhow::anyhow;
use async_channel::unbounded;
use async_trait::async_trait;
use chrono::Utc;
use itertools::Itertools;
use jsonrpc_core::futures::StreamExt;
use jsonrpc_core_client::transports::{http, ws};
use log::*;
use solana_account_decoder::{UiAccount, UiAccountEncoding};
use solana_client::{
    rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
    rpc_response::{OptionalContext, Response, RpcKeyedAccount},
};
use solana_program::{slot_history::Slot, stake_history::Epoch};
use solana_rpc::rpc_pubsub::RpcSolPubSubClient;
use solana_sdk::{
    account::{Account, AccountSharedData, ReadableAccount, WritableAccount},
    commitment_config::{CommitmentConfig, CommitmentLevel},
    compute_budget::ComputeBudgetInstruction,
    hash::Hash,
    instruction::Instruction,
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    transaction::Transaction,
};
use std::{
    collections::{BTreeSet, HashMap},
    str::FromStr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc::UnboundedSender, RwLock},
    task::JoinHandle,
};

#[derive(Debug, Clone)]
pub struct KeeperConfig {
    pub program_id: Pubkey,
    pub rpc_url: String,
    pub websocket_url: String,
}

pub fn start(
    config: KeeperConfig,
    blockhash: Arc<RwLock<Hash>>,
    current_slot: Arc<AtomicU64>,
    openbook_config: &Obv2Config,
    identity: &Keypair,
    prioritization_fee: u64,
    tx_rx: UnboundedSender<(Transaction, TransactionSendRecord)>,
) -> Vec<JoinHandle<()>> {
    let (instruction_sender, instruction_receiver) = unbounded::<(Pubkey, Vec<Instruction>)>();
    let identity = Keypair::from_bytes(identity.to_bytes().as_slice()).unwrap();
    let t1 = tokio::spawn(async move {
        info!(
            "crank-tx-sender signing with keypair pk={:?}",
            identity.pubkey()
        );

        loop {
            if let Ok((market, mut ixs)) = instruction_receiver.recv().await {
                // add priority fees
                ixs.push(ComputeBudgetInstruction::set_compute_unit_price(
                    prioritization_fee,
                ));

                let tx = Transaction::new_signed_with_payer(
                    &ixs,
                    Some(&identity.pubkey()),
                    &[&identity],
                    *blockhash.read().await,
                );

                let tx_send_record = TransactionSendRecord {
                    signature: tx.signatures[0],
                    sent_at: Utc::now(),
                    sent_slot: current_slot.load(Ordering::Acquire),
                    market: Some(market),
                    priority_fees: prioritization_fee,
                    user: None,
                    is_consume_event: true,
                };

                let _ = tx_rx.send((tx, tx_send_record));
            }
        }
    });

    let event_queues = openbook_config
        .markets
        .iter()
        .map(|x| x.event_queue.clone())
        .collect_vec();
    let openbook_config = openbook_config.clone();

    let t2 = tokio::spawn(async move {
        let routes = vec![AccountWriteRoute {
            matched_pubkeys: event_queues.clone(),
            sink: Arc::new(OpenbookV2CrankSink::new(
                openbook_config,
                instruction_sender,
                config.program_id,
            )),
            timeout_interval: Duration::default(),
        }];

        let filter_config = FilterConfig {
            program_ids: vec![config.program_id.to_string()],
            account_ids: event_queues.iter().map(|x| x.to_string()).collect_vec(),
        };

        info!("matched_pks={:?}", routes[0].matched_pubkeys);

        let (account_write_queue_sender, slot_queue_sender) =
            init(routes).expect("filter initializes");

        info!(
            "start processing websocket events program_id={:?} ws_url={:?}",
            config.program_id, config.websocket_url
        );

        process_events(
            &SourceConfig {
                dedup_queue_size: 0,
                rpc_http_url: config.rpc_url,
                program_id: config.program_id.to_string(),
                rpc_ws_url: config.websocket_url,
            },
            &filter_config,
            account_write_queue_sender,
            slot_queue_sender,
        )
        .await;
    });

    vec![t1, t2]
}

/// Code copied from mango-feeds

#[derive(Clone, Debug)]
pub struct AccountData {
    pub slot: u64,
    pub write_version: u64,
    pub account: AccountSharedData,
}

#[async_trait]
pub trait AccountWriteSink {
    async fn process(&self, pubkey: &Pubkey, account: &AccountData) -> Result<(), String>;
}

#[derive(Clone)]
pub struct AccountWriteRoute {
    pub matched_pubkeys: Vec<Pubkey>,
    pub sink: Arc<dyn AccountWriteSink + Send + Sync>,
    pub timeout_interval: Duration,
}

#[derive(Clone, Debug)]
pub struct SourceConfig {
    pub dedup_queue_size: usize,
    pub rpc_http_url: String,
    pub program_id: String,
    pub rpc_ws_url: String,
}

#[derive(Clone, Debug)]
pub struct FilterConfig {
    pub program_ids: Vec<String>,
    pub account_ids: Vec<String>,
}

enum WebsocketMessage {
    SingleUpdate(Response<RpcKeyedAccount>),
    SnapshotUpdate((Slot, Vec<(String, Option<UiAccount>)>)),
    SlotUpdate(Arc<solana_client::rpc_response::SlotUpdate>),
}

#[derive(Clone, Debug)]
pub struct SlotUpdate {
    pub slot: u64,
    pub parent: Option<u64>,
    pub status: CommitmentLevel,
}

#[derive(Clone, PartialEq, Debug)]
pub struct AccountWrite {
    pub pubkey: Pubkey,
    pub slot: u64,
    pub write_version: u64,
    pub lamports: u64,
    pub owner: Pubkey,
    pub executable: bool,
    pub rent_epoch: u64,
    pub data: Vec<u8>,
    pub is_selected: bool,
}

impl AccountWrite {
    fn from(pubkey: Pubkey, slot: u64, write_version: u64, account: Account) -> AccountWrite {
        AccountWrite {
            pubkey,
            slot,
            write_version,
            lamports: account.lamports,
            owner: account.owner,
            executable: account.executable,
            rent_epoch: account.rent_epoch,
            data: account.data,
            is_selected: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SlotData {
    pub slot: u64,
    pub parent: Option<u64>,
    pub status: CommitmentLevel,
    pub chain: u64, // the top slot that this is in a chain with. uncles will have values < tip
}

#[derive(Clone, Debug)]
struct AccountWriteRecord {
    slot: u64,
    write_version: u64,
    timestamp: Instant,
}

pub fn init(
    routes: Vec<AccountWriteRoute>,
) -> anyhow::Result<(
    async_channel::Sender<AccountWrite>,
    async_channel::Sender<SlotUpdate>,
)> {
    let (account_write_queue_sender, account_write_queue_receiver) =
        async_channel::unbounded::<AccountWrite>();

    // Slot updates flowing from the outside into this processing thread. From
    // there the AccountWriteRoute::sink() callback is triggered.
    let (slot_queue_sender, slot_queue_receiver) = async_channel::unbounded::<SlotUpdate>();

    let mut chain_data = ChainData::new();

    let mut last_updated = HashMap::<String, AccountWriteRecord>::new();

    let all_queue_pks: BTreeSet<Pubkey> = routes
        .iter()
        .flat_map(|r| r.matched_pubkeys.iter())
        .map(|pk| pk.clone())
        .collect();

    // update handling thread, reads both slots and account updates
    tokio::spawn(async move {
        loop {
            tokio::select! {
                Ok(account_write) = account_write_queue_receiver.recv() => {
                    if !all_queue_pks.contains(&account_write.pubkey) {
                        trace!("account write skipped {:?}", account_write.pubkey);
                        continue;
                    } else {
                        trace!("account write processed {:?}", account_write.pubkey);
                    }

                    chain_data.update_account(
                        account_write.pubkey,
                        AccountData {
                            slot: account_write.slot,
                            write_version: account_write.write_version,
                            account: WritableAccount::create(
                                account_write.lamports,
                                account_write.data.clone(),
                                account_write.owner,
                                account_write.executable,
                                account_write.rent_epoch as Epoch,
                            ),
                        },
                    );
                }
                Ok(slot_update) = slot_queue_receiver.recv() => {
                    trace!("slot update processed {:?}", slot_update);
                    chain_data.update_slot(SlotData {
                        slot: slot_update.slot,
                        parent: slot_update.parent,
                        status: slot_update.status,
                        chain: 0,
                    });

                }
                else => {
                    warn!("channels closed, filter shutting down pks={all_queue_pks:?}");
                    break;
                }

            }

            for route in routes.iter() {
                for pk in route.matched_pubkeys.iter() {
                    match chain_data.account(&pk) {
                        Ok(account_info) => {
                            let pk_b58 = pk.to_string();
                            if let Some(record) = last_updated.get(&pk_b58) {
                                let is_unchanged = account_info.slot == record.slot
                                    && account_info.write_version == record.write_version;
                                let is_throttled =
                                    record.timestamp.elapsed() < route.timeout_interval;
                                if is_unchanged || is_throttled {
                                    trace!("skipped is_unchanged={is_unchanged} is_throttled={is_throttled} pk={pk_b58}");
                                    continue;
                                }
                            };

                            match route.sink.process(pk, account_info).await {
                                Ok(()) => {
                                    // todo: metrics
                                    last_updated.insert(
                                        pk_b58.clone(),
                                        AccountWriteRecord {
                                            slot: account_info.slot,
                                            write_version: account_info.write_version,
                                            timestamp: Instant::now(),
                                        },
                                    );
                                }
                                Err(skip_reason) => {
                                    debug!("sink process skipped reason={skip_reason} pk={pk_b58}");
                                    // todo: metrics
                                }
                            }
                        }
                        Err(_) => {
                            debug!("could not find pk in chain data pk={:?}", pk);
                            // todo: metrics
                        }
                    }
                }
            }
        }
    });

    Ok((account_write_queue_sender, slot_queue_sender))
}

pub async fn process_events(
    config: &SourceConfig,
    filter_config: &FilterConfig,
    account_write_queue_sender: async_channel::Sender<AccountWrite>,
    slot_queue_sender: async_channel::Sender<SlotUpdate>,
) {
    // Subscribe to program account updates websocket
    let (update_sender, update_receiver) = async_channel::unbounded::<WebsocketMessage>();
    let config = config.clone();
    let filter_config = filter_config.clone();
    tokio::spawn(async move {
        // if the websocket disconnects, we get no data in a while etc, reconnect and try again
        loop {
            let out = feed_data(&config, &filter_config, update_sender.clone());
            let _ = out.await;
        }
    });

    //
    // The thread that pulls updates and forwards them to postgres
    //

    // copy websocket updates into the postgres account write queue
    loop {
        let update = update_receiver.recv().await.unwrap();
        trace!("got update message");

        match update {
            WebsocketMessage::SingleUpdate(update) => {
                trace!("single update");
                let account: Account = update.value.account.decode().unwrap();
                let pubkey = Pubkey::from_str(&update.value.pubkey).unwrap();
                account_write_queue_sender
                    .send(AccountWrite::from(pubkey, update.context.slot, 0, account))
                    .await
                    .expect("send success");
            }
            WebsocketMessage::SnapshotUpdate((slot, accounts)) => {
                trace!("snapshot update {slot}");
                for (pubkey, account) in accounts {
                    if let Some(account) = account {
                        let pubkey = Pubkey::from_str(&pubkey).unwrap();
                        account_write_queue_sender
                            .send(AccountWrite::from(
                                pubkey,
                                slot,
                                0,
                                account.decode().unwrap(),
                            ))
                            .await
                            .expect("send success");
                    }
                }
            }
            WebsocketMessage::SlotUpdate(update) => {
                trace!("slot update");
                let message = match *update {
                    solana_client::rpc_response::SlotUpdate::CreatedBank {
                        slot, parent, ..
                    } => Some(SlotUpdate {
                        slot,
                        parent: Some(parent),
                        status: CommitmentLevel::Processed,
                    }),
                    solana_client::rpc_response::SlotUpdate::OptimisticConfirmation {
                        slot,
                        ..
                    } => Some(SlotUpdate {
                        slot,
                        parent: None,
                        status: CommitmentLevel::Confirmed,
                    }),
                    solana_client::rpc_response::SlotUpdate::Root { slot, .. } => {
                        Some(SlotUpdate {
                            slot,
                            parent: None,
                            status: CommitmentLevel::Finalized,
                        })
                    }
                    _ => None,
                };
                if let Some(message) = message {
                    slot_queue_sender.send(message).await.expect("send success");
                }
            }
        }
    }
}

trait AnyhowWrap {
    type Value;
    fn map_err_anyhow(self) -> anyhow::Result<Self::Value>;
}

impl<T, E: std::fmt::Debug> AnyhowWrap for Result<T, E> {
    type Value = T;
    fn map_err_anyhow(self) -> anyhow::Result<Self::Value> {
        self.map_err(|err| anyhow::anyhow!("{:?}", err))
    }
}

async fn feed_data(
    config: &SourceConfig,
    filter_config: &FilterConfig,
    sender: async_channel::Sender<WebsocketMessage>,
) -> anyhow::Result<()> {
    debug!("feed_data {config:?}");

    let program_id = Pubkey::from_str(&config.program_id)?;
    let snapshot_duration = Duration::from_secs(300);

    let connect =
        ws::try_connect::<RpcSolPubSubClient>(&config.rpc_ws_url).expect("should connect to ws");
    let client = connect.await.expect("should connect to ws client");

    let account_info_config = RpcAccountInfoConfig {
        encoding: Some(UiAccountEncoding::Base64),
        commitment: Some(CommitmentConfig::processed()),
        data_slice: None,
        min_context_slot: None,
    };
    let program_accounts_config = RpcProgramAccountsConfig {
        filters: None,
        with_context: Some(true),
        account_config: account_info_config.clone(),
    };

    let mut update_sub = client
        .program_subscribe(
            program_id.to_string(),
            Some(program_accounts_config.clone()),
        )
        .map_err_anyhow()?;
    let mut slot_sub = client.slots_updates_subscribe().map_err_anyhow()?;

    let mut last_snapshot = Instant::now().checked_sub(snapshot_duration).unwrap();

    loop {
        // occasionally cause a new snapshot to be produced
        // including the first time
        if last_snapshot + snapshot_duration <= Instant::now() {
            let snapshot = get_snapshot(config.rpc_http_url.clone(), filter_config).await;
            if let Ok((slot, accounts)) = snapshot {
                debug!(
                    "fetched new snapshot slot={slot} len={:?} time={:?}",
                    accounts.len(),
                    Instant::now() - snapshot_duration - last_snapshot
                );
                sender
                    .send(WebsocketMessage::SnapshotUpdate((slot, accounts)))
                    .await
                    .expect("sending must succeed");
            } else {
                error!("failed to parse snapshot")
            }
            last_snapshot = Instant::now();
        }

        tokio::select! {
            account = update_sub.next() => {
                match account {
                    Some(account) => {
                        sender.send(WebsocketMessage::SingleUpdate(account.map_err_anyhow()?)).await.expect("sending must succeed");
                    },
                    None => {
                        warn!("account stream closed");
                        return Ok(());
                    },
                }
            },
            slot_update = slot_sub.next() => {
                match slot_update {
                    Some(slot_update) => {
                        sender.send(WebsocketMessage::SlotUpdate(slot_update.map_err_anyhow()?)).await.expect("sending must succeed");
                    },
                    None => {
                        warn!("slot update stream closed");
                        return Ok(());
                    },
                }
            },
            _ = tokio::time::sleep(Duration::from_secs(60)) => {
                warn!("websocket timeout");
                return Ok(())
            }
        }
    }
}

use solana_rpc::rpc::rpc_accounts::AccountsDataClient;
pub async fn get_snapshot_gpa(
    rpc_http_url: String,
    program_id: String,
) -> anyhow::Result<OptionalContext<Vec<RpcKeyedAccount>>> {
    let rpc_client = http::connect::<AccountsDataClient>(&rpc_http_url)
        .await
        .map_err_anyhow()?;

    let account_info_config = RpcAccountInfoConfig {
        encoding: Some(UiAccountEncoding::Base64),
        commitment: Some(CommitmentConfig::finalized()),
        data_slice: None,
        min_context_slot: None,
    };
    let program_accounts_config = RpcProgramAccountsConfig {
        filters: None,
        with_context: Some(true),
        account_config: account_info_config.clone(),
    };

    info!("requesting snapshot {}", program_id);
    let account_snapshot = rpc_client
        .get_program_accounts(program_id.clone(), Some(program_accounts_config.clone()))
        .await
        .map_err_anyhow()?;
    info!("snapshot received {}", program_id);
    Ok(account_snapshot)
}

pub async fn get_snapshot_gma(
    rpc_http_url: String,
    ids: Vec<String>,
) -> anyhow::Result<solana_client::rpc_response::Response<Vec<Option<UiAccount>>>> {
    let rpc_client = http::connect::<AccountsDataClient>(&rpc_http_url)
        .await
        .map_err_anyhow()?;

    let account_info_config = RpcAccountInfoConfig {
        encoding: Some(UiAccountEncoding::Base64),
        commitment: Some(CommitmentConfig::finalized()),
        data_slice: None,
        min_context_slot: None,
    };

    info!("requesting snapshot {:?}", ids);
    let account_snapshot = rpc_client
        .get_multiple_accounts(ids.clone(), Some(account_info_config))
        .await
        .map_err_anyhow()?;
    info!("snapshot received {:?}", ids);
    Ok(account_snapshot)
}

pub async fn get_snapshot(
    rpc_http_url: String,
    filter_config: &FilterConfig,
) -> anyhow::Result<(Slot, Vec<(String, Option<UiAccount>)>)> {
    if !filter_config.account_ids.is_empty() {
        let response =
            get_snapshot_gma(rpc_http_url.clone(), filter_config.account_ids.clone()).await;
        if let Ok(snapshot) = response {
            let accounts: Vec<(String, Option<UiAccount>)> = filter_config
                .account_ids
                .iter()
                .zip(snapshot.value)
                .map(|x| (x.0.clone(), x.1))
                .collect();
            Ok((snapshot.context.slot, accounts))
        } else {
            Err(anyhow!("invalid gma response {:?}", response))
        }
    } else if !filter_config.program_ids.is_empty() {
        let response =
            get_snapshot_gpa(rpc_http_url.clone(), filter_config.program_ids[0].clone()).await;
        if let Ok(OptionalContext::Context(snapshot)) = response {
            let accounts: Vec<(String, Option<UiAccount>)> = snapshot
                .value
                .iter()
                .map(|x| {
                    let deref = x.clone();
                    (deref.pubkey, Some(deref.account))
                })
                .collect();
            Ok((snapshot.context.slot, accounts))
        } else {
            Err(anyhow!("invalid gpa response {:?}", response))
        }
    } else {
        Err(anyhow!("invalid filter_config"))
    }
}

/// Track slots and account writes
///
/// - use account() to retrieve the current best data for an account.
/// - update_from_snapshot() and update_from_websocket() update the state for new messages
pub struct ChainData {
    /// only slots >= newest_rooted_slot are retained
    slots: HashMap<u64, SlotData>,
    /// writes to accounts, only the latest rooted write an newer are retained
    accounts: HashMap<Pubkey, Vec<AccountData>>,
    newest_rooted_slot: u64,
    newest_processed_slot: u64,
    best_chain_slot: u64,
    account_versions_stored: usize,
    account_bytes_stored: usize,
}

impl ChainData {
    pub fn new() -> Self {
        Self {
            slots: HashMap::new(),
            accounts: HashMap::new(),
            newest_rooted_slot: 0,
            newest_processed_slot: 0,
            best_chain_slot: 0,
            account_versions_stored: 0,
            account_bytes_stored: 0,
        }
    }

    pub fn update_slot(&mut self, new_slot: SlotData) {
        let new_processed_head = new_slot.slot > self.newest_processed_slot;
        if new_processed_head {
            self.newest_processed_slot = new_slot.slot;
        }

        let new_rooted_head = new_slot.slot > self.newest_rooted_slot
            && new_slot.status == CommitmentLevel::Finalized;
        if new_rooted_head {
            self.newest_rooted_slot = new_slot.slot;
        }

        // Use the highest slot that has a known parent as best chain
        // (sometimes slots OptimisticallyConfirm before we even know the parent!)
        let new_best_chain = new_slot.parent.is_some() && new_slot.slot > self.best_chain_slot;
        if new_best_chain {
            self.best_chain_slot = new_slot.slot;
        }

        let mut parent_update = false;

        use std::collections::hash_map::Entry;
        match self.slots.entry(new_slot.slot) {
            Entry::Vacant(v) => {
                v.insert(new_slot);
            }
            Entry::Occupied(o) => {
                let v = o.into_mut();
                parent_update = v.parent != new_slot.parent && new_slot.parent.is_some();
                v.parent = v.parent.or(new_slot.parent);
                // Never decrease the slot status
                if v.status == CommitmentLevel::Processed
                    || new_slot.status == CommitmentLevel::Finalized
                {
                    v.status = new_slot.status;
                }
            }
        };

        if new_best_chain || parent_update {
            // update the "chain" field down to the first rooted slot
            let mut slot = self.best_chain_slot;
            loop {
                if let Some(data) = self.slots.get_mut(&slot) {
                    data.chain = self.best_chain_slot;
                    if data.status == CommitmentLevel::Finalized {
                        break;
                    }
                    if let Some(parent) = data.parent {
                        slot = parent;
                        continue;
                    }
                }
                break;
            }
        }

        if new_rooted_head {
            // for each account, preserve only writes > newest_rooted_slot, or the newest
            // rooted write
            self.account_versions_stored = 0;
            self.account_bytes_stored = 0;

            for (_, writes) in self.accounts.iter_mut() {
                let newest_rooted_write = writes
                    .iter()
                    .rev()
                    .find(|w| {
                        w.slot <= self.newest_rooted_slot
                            && self
                                .slots
                                .get(&w.slot)
                                .map(|s| {
                                    // sometimes we seem not to get notifications about slots
                                    // getting rooted, hence assume non-uncle slots < newest_rooted_slot
                                    // are rooted too
                                    s.status == CommitmentLevel::Finalized
                                        || s.chain == self.best_chain_slot
                                })
                                // preserved account writes for deleted slots <= newest_rooted_slot
                                // are expected to be rooted
                                .unwrap_or(true)
                    })
                    .map(|w| w.slot)
                    // no rooted write found: produce no effect, since writes > newest_rooted_slot are retained anyway
                    .unwrap_or(self.newest_rooted_slot + 1);
                writes
                    .retain(|w| w.slot == newest_rooted_write || w.slot > self.newest_rooted_slot);
                self.account_versions_stored += writes.len();
                self.account_bytes_stored +=
                    writes.iter().map(|w| w.account.data().len()).sum::<usize>()
            }

            // now it's fine to drop any slots before the new rooted head
            // as account writes for non-rooted slots before it have been dropped
            self.slots.retain(|s, _| *s >= self.newest_rooted_slot);
        }
    }

    pub fn update_account(&mut self, pubkey: Pubkey, account: AccountData) {
        use std::collections::hash_map::Entry;
        match self.accounts.entry(pubkey) {
            Entry::Vacant(v) => {
                self.account_versions_stored += 1;
                self.account_bytes_stored += account.account.data().len();
                v.insert(vec![account]);
            }
            Entry::Occupied(o) => {
                let v = o.into_mut();
                // v is ordered by slot ascending. find the right position
                // overwrite if an entry for the slot already exists, otherwise insert
                let rev_pos = v
                    .iter()
                    .rev()
                    .position(|d| d.slot <= account.slot)
                    .unwrap_or(v.len());
                let pos = v.len() - rev_pos;
                if pos < v.len() && v[pos].slot == account.slot {
                    if v[pos].write_version <= account.write_version {
                        v[pos] = account;
                    }
                } else {
                    self.account_versions_stored += 1;
                    self.account_bytes_stored += account.account.data().len();
                    v.insert(pos, account);
                }
            }
        };
    }

    fn is_account_write_live(&self, write: &AccountData) -> bool {
        self.slots
            .get(&write.slot)
            // either the slot is rooted or in the current chain
            .map(|s| {
                s.status == CommitmentLevel::Finalized
                    || s.chain == self.best_chain_slot
                    || write.slot > self.best_chain_slot
            })
            // if the slot can't be found but preceeds newest rooted, use it too (old rooted slots are removed)
            .unwrap_or(write.slot <= self.newest_rooted_slot || write.slot > self.best_chain_slot)
    }

    /// Ref to the most recent live write of the pubkey
    pub fn account<'a>(&'a self, pubkey: &Pubkey) -> anyhow::Result<&'a AccountData> {
        self.accounts
            .get(pubkey)
            .ok_or_else(|| anyhow::anyhow!("account {} not found", pubkey))?
            .iter()
            .rev()
            .find(|w| self.is_account_write_live(w))
            .ok_or_else(|| anyhow::anyhow!("account {} has no live data", pubkey))
    }
}
