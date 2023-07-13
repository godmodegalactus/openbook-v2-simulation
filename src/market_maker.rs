use std::{sync::Arc, time::Duration};

use crate::{
    cli::Args,
    openbook_config::{Obv2Config, Obv2Market, Obv2User},
};
use anchor_lang::{InstructionData, ToAccountMetas};
use rand::{rngs::StdRng, Rng, SeedableRng};
use solana_sdk::hash::Hash;
use solana_sdk::{
    instruction::Instruction, message::Message, pubkey::Pubkey, signer::Signer,
    transaction::Transaction,
};
use tokio::{
    sync::{mpsc::UnboundedSender, RwLock},
    task::JoinHandle,
    time::Instant,
};

fn create_market_making_instructions(
    user: &Obv2User,
    market: &Obv2Market,
    program_id: Pubkey,
    client_id: u64,
    offset: i64,
    size: i64,
) -> Vec<Instruction> {
    let open_orders_account = user
        .open_orders
        .iter()
        .find(|x| x.market == market.market_pk)
        .map(|x| x.open_orders.clone())
        .expect("should exists");

    // cancel all previous orders
    let cancel_order_instruction = {
        let accounts = openbook_v2::accounts::CancelAllOrders {
            asks: market.asks,
            bids: market.bids,
            market: market.market_pk,
            open_orders_account,
            owner: user.secret.pubkey(),
        };
        let accounts_meta = accounts.to_account_metas(None);
        let instruction_data = openbook_v2::instruction::CancelAllOrders {
            limit: 255,
            side_option: None,
        };
        Instruction::new_with_bytes(
            program_id,
            instruction_data.data().as_slice(),
            accounts_meta,
        )
    };

    // place bid order
    let place_bid_order = {
        let accounts = openbook_v2::accounts::PlaceOrder {
            asks: market.asks,
            bids: market.bids,
            event_queue: market.event_queue,
            market: market.market_pk,
            market_vault: market.base_vault,
            open_orders_account,
            open_orders_admin: None,
            oracle: market.oracle,
            owner_or_delegate: user.secret.pubkey(),
            token_deposit_account: market.quote_vault,
            system_program: anchor_client::solana_sdk::system_program::id(),
            token_program: anchor_spl::token::ID,
        };

        let instruction_data = openbook_v2::instruction::PlaceOrder {
            order_type: openbook_v2::state::PlaceOrderType::Limit,
            limit: 255,
            client_order_id: client_id,
            max_base_lots: size,
            expiry_timestamp: 0,
            max_quote_lots_including_fees: i64::MAX,
            price_lots: market.price as i64 + offset,
            side: openbook_v2::state::Side::Bid,
            self_trade_behavior: openbook_v2::state::SelfTradeBehavior::DecrementTake,
        };
        Instruction::new_with_bytes(
            program_id,
            instruction_data.data().as_slice(),
            accounts.to_account_metas(None),
        )
    };

    // place ask order
    let place_ask_order = {
        let accounts = openbook_v2::accounts::PlaceOrder {
            asks: market.asks,
            bids: market.bids,
            event_queue: market.event_queue,
            market: market.market_pk,
            market_vault: market.quote_vault,
            open_orders_account,
            open_orders_admin: None,
            oracle: market.oracle,
            owner_or_delegate: user.secret.pubkey(),
            token_deposit_account: market.base_vault,
            system_program: anchor_client::solana_sdk::system_program::id(),
            token_program: anchor_spl::token::ID,
        };

        let instruction_data = openbook_v2::instruction::PlaceOrder {
            order_type: openbook_v2::state::PlaceOrderType::Limit,
            limit: 255,
            client_order_id: client_id,
            max_base_lots: i64::MAX,
            expiry_timestamp: 0,
            max_quote_lots_including_fees: size,
            price_lots: market.price as i64 - offset,
            side: openbook_v2::state::Side::Ask,
            self_trade_behavior: openbook_v2::state::SelfTradeBehavior::DecrementTake,
        };
        Instruction::new_with_bytes(
            program_id,
            instruction_data.data().as_slice(),
            accounts.to_account_metas(None),
        )
    };

    vec![cancel_order_instruction, place_bid_order, place_ask_order]
}

async fn start_market_making(
    user: Obv2User,
    market: Obv2Market,
    program_id: Pubkey,
    transaction_send_channel: UnboundedSender<Transaction>,
    block_hash_rw: Arc<RwLock<Hash>>,
    quotes_per_second: u64,
    seed: u64,
) {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut client_id = 0;
    loop {
        let instant = Instant::now();
        for _ in 0..quotes_per_second {
            let offset: i64 = rng.gen::<i64>() % 100;
            let size: u64 = rng.gen::<u64>() % 1000 + 10;
            let instructions = create_market_making_instructions(
                &user,
                &market,
                program_id,
                client_id,
                offset,
                size as i64,
            );
            let recent_blockhash = *block_hash_rw.read().await;

            let message = Message::new(&instructions, Some(&user.secret.pubkey()));
            let tx = Transaction::new(&[user.secret.as_ref()], message, recent_blockhash);
            let signature = tx.signatures[0];
            match transaction_send_channel.send(tx) {
                Ok(_) => {
                    log::trace!("successfully sent {} on channel", signature);
                }
                Err(e) => {
                    log::error!("sending of channel failed {}", e);
                }
            }

            client_id += 1;
        }

        let time_elapsed = instant.elapsed();
        if time_elapsed < Duration::from_secs(1) {
            tokio::time::sleep(Duration::from_secs(1) - time_elapsed).await;
        }
    }
}

pub fn start_market_makers(
    args: &Args,
    config: &Obv2Config,
    program_id: &Pubkey,
    transaction_send_channel: UnboundedSender<Transaction>,
    block_hash_rw: Arc<RwLock<Hash>>,
) -> Vec<JoinHandle<()>> {
    let mut tasks = vec![];
    let mut seed = 0;
    let quotes_per_second = args.quotes_per_seconds;
    for user in &config.users {
        for market in &config.markets {
            let user = user.clone();
            let market = market.clone();
            let transaction_send_channel = transaction_send_channel.clone();
            let block_hash_rw = block_hash_rw.clone();
            let program_id = program_id.clone();
            let task = tokio::spawn(async move {
                start_market_making(
                    user,
                    market,
                    program_id,
                    transaction_send_channel,
                    block_hash_rw,
                    quotes_per_second,
                    seed,
                )
                .await;
            });

            seed += 1;

            tasks.push(task);
        }
    }
    tasks
}
