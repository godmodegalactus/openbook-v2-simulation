use std::{
    collections::{BTreeMap, HashSet},
};

use async_trait::async_trait;
use bytemuck::cast_ref;
use openbook_v2::state::{EventQueue, EventType, FillEvent, OutEvent};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_sdk::account::ReadableAccount;

use crate::{
    crank::{AccountData, AccountWriteSink},
    openbook_config::Obv2Config,
};
use anchor_lang::AccountDeserialize;
use anchor_lang::{InstructionData, ToAccountMetas};
use async_channel::Sender;

const MAX_BACKLOG: usize = 2;
const MAX_EVENTS_PER_TX: usize = 50;
const MAX_ACCS_PER_TX: usize = 24;

pub struct OpenbookV2CrankSink {
    instruction_sender: Sender<(Pubkey, Vec<Instruction>)>,
    map_event_q_to_market: BTreeMap<Pubkey, Pubkey>,
    program_id: Pubkey,
}

impl OpenbookV2CrankSink {
    pub fn new(
        obv2_config: Obv2Config,
        instruction_sender: Sender<(Pubkey, Vec<Instruction>)>,
        program_id: Pubkey,
    ) -> Self {
        let mut map_event_q_to_market = BTreeMap::new();
        for market in &obv2_config.markets {
            map_event_q_to_market.insert(market.event_queue, market.market_pk);
        }
        Self {
            instruction_sender,
            map_event_q_to_market,
            program_id,
        }
    }
}

#[async_trait]
impl AccountWriteSink for OpenbookV2CrankSink {
    async fn process(
        &self,
        pk: &solana_sdk::pubkey::Pubkey,
        account: &AccountData,
    ) -> Result<(), String> {
        let account = &account.account;

        let (ix, mkt_pk): (Result<Instruction, String>, Pubkey) = {
            let mut header_data: &[u8] = account.data();

            let event_queue: EventQueue = EventQueue::try_deserialize(&mut header_data)
                .expect("event queue should be correctly deserailizable");

            // only crank if at least 1 fill or a sufficient events of other categories are buffered
            let contains_fill_events = event_queue
                .iter()
                .find(|e| e.0.event_type == EventType::Fill as u8)
                .is_some();
            let len = event_queue.iter().count();
            let has_backlog = len > MAX_BACKLOG;
            let seq_num = event_queue.header.seq_num;
            log::debug!("evq {pk:?} seq_num={seq_num} len={len} contains_fill_events={contains_fill_events} has_backlog={has_backlog}");

            if !contains_fill_events && !has_backlog {
                return Err("throttled".into());
            }

            let mut events_accounts = HashSet::new();
            event_queue.iter().take(MAX_EVENTS_PER_TX).for_each(|e| {
                if events_accounts.len() < MAX_ACCS_PER_TX {
                    match EventType::try_from(e.0.event_type).expect("openbook v2 event") {
                        EventType::Fill => {
                            let fill: &FillEvent = cast_ref(e.0);
                            events_accounts.insert(fill.maker);
                            events_accounts.insert(fill.taker);
                        }
                        EventType::Out => {
                            let out: &OutEvent = cast_ref(e.0);
                            events_accounts.insert(out.owner);
                        }
                    }
                }
            });

            let mkt_pk = self
                .map_event_q_to_market
                .get(&pk)
                .expect(&format!("{pk:?} is a known public key"));

            let mut accounts_meta = openbook_v2::accounts::ConsumeEvents {
                consume_events_admin: None,
                event_queue: pk.clone(),
                market: mkt_pk.clone(),
            }
            .to_account_metas(None);

            for event_account in events_accounts {
                accounts_meta.push(AccountMeta {
                    pubkey: event_account,
                    is_signer: false,
                    is_writable: true,
                })
            }

            let instruction_data = openbook_v2::instruction::ConsumeEvents {
                limit: MAX_EVENTS_PER_TX,
            };

            let ix = Instruction::new_with_bytes(
                self.program_id,
                instruction_data.data().as_slice(),
                accounts_meta,
            );
            (Ok(ix), mkt_pk.clone())
        };

        if let Err(e) = self.instruction_sender.send((mkt_pk, vec![ix?])).await {
            return Err(e.to_string());
        }

        Ok(())
    }
}
