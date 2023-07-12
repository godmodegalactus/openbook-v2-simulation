// simlar to json config just the Keypairs are changed to Pubkeys

use solana_program::pubkey::Pubkey;
use solana_sdk::{signature::Keypair, signer::Signer};
use std::sync::Arc;
use crate::json_config::{Config, ProgramData, User, TokenAccountData, OpenOrders, Market};
use itertools::Itertools;

#[derive(Clone, Debug)]
pub struct Obv2ProgramData {
    pub name: String,
    pub program_id: Pubkey,
}

#[derive(Clone, Debug)]
pub struct Obv2User {
    pub secret: Arc<Keypair>,
    pub token_data: Vec<Obv2TokenAccountData>,
    pub open_orders: Vec<Obv2OpenOrders>,
}

#[derive(Clone, Debug)]
pub struct Obv2TokenAccountData {
    pub mint: Pubkey,
    pub token_account: Pubkey,
}

#[derive(Clone, Debug)]
pub struct Obv2OpenOrders {
    pub market: Pubkey,
    pub open_orders: Pubkey,
}

#[derive(Clone, Debug)]
pub struct Obv2Market {
    pub name: String,
    pub admin: Arc<Keypair>,
    pub market_pk: Pubkey,
    pub oracle: Pubkey,
    pub asks: Pubkey,
    pub bids: Pubkey,
    pub event_queue: Pubkey,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub market_index: usize,
    pub price: u64,
}

#[derive(Clone, Debug)]
pub struct Obv2Config {
    pub programs: Vec<Obv2ProgramData>,
    pub users: Vec<Obv2User>,
    pub mints: Vec<Pubkey>,
    pub markets: Vec<Obv2Market>,
}

pub fn convert_to_pk(key: &String) -> Pubkey {
    Pubkey::try_from(key.as_str()).expect("Should be convertible to pubkey")
}

impl From<&Config> for Obv2Config {
    fn from(value: &Config) -> Self {
        Self {
            programs : value.programs.iter().map(|x| Obv2ProgramData::try_from(x).expect("Program should be Pubkey")).collect_vec(),
            users: value.users.iter().map(|x| Obv2User::try_from(x).expect("User should be Pubkey")).collect_vec(),
            mints: value.mints.iter().map(|x| convert_to_pk(x)).collect_vec(),
            markets: value.markets.iter().map(|x| Obv2Market::try_from(x).expect("Market should be Pubkey")).collect_vec(),
        }
    }
}

impl From<&ProgramData> for Obv2ProgramData {
    fn from(value: &ProgramData) -> Self {
        Self { name: value.name.clone(), program_id: convert_to_pk(&value.program_id) }
    }
}


pub fn get_keypair(secret: &Vec<u8>) -> Keypair {
    Keypair::from_bytes(&secret).unwrap()
}

impl From<&TokenAccountData> for Obv2TokenAccountData {
    fn from(value: &TokenAccountData) -> Self {
        Self {
            mint: convert_to_pk(&value.mint),
            token_account: convert_to_pk(&value.token_account),
        }
    }
}

impl From<&OpenOrders> for Obv2OpenOrders {
    fn from(value: &OpenOrders) -> Self {
        Self { market: convert_to_pk(&value.market), open_orders: convert_to_pk(&value.open_orders) }
    }
}

impl From<&Market> for Obv2Market {
    fn from(value: &Market) -> Self {
        Self { 
            name: value.name.clone(), 
            admin: Arc::new(get_keypair(&value.admin)), 
            market_pk: convert_to_pk(&value.market_pk), 
            oracle: convert_to_pk(&value.oracle), 
            asks: convert_to_pk(&value.asks), 
            bids: convert_to_pk(&value.bids), 
            event_queue: convert_to_pk(&value.event_queue), 
            base_vault: convert_to_pk(&value.base_vault), 
            quote_vault: convert_to_pk(&value.quote_vault), 
            base_mint: convert_to_pk(&value.base_mint), 
            quote_mint: convert_to_pk(&value.quote_mint), 
            market_index: value.market_index,
            price: value.price,
        }
    }
}

impl From<&User> for Obv2User {
    fn from(value: &User) -> Self {
        Self {
            secret: Arc::new(get_keypair(&value.secret)),
            open_orders: value.open_orders.iter().map(|x| Obv2OpenOrders::try_from(x).expect("OpenOrders should be Pubkey")).collect_vec(),
            token_data: value.token_data.iter().map(|x| Obv2TokenAccountData::try_from(x).expect("OpenOrders should be Pubkey")).collect_vec(),
        }
    }
}

impl Signer for Obv2User {
    fn try_pubkey(&self) -> Result<solana_sdk::pubkey::Pubkey, solana_sdk::signer::SignerError> {
        Ok(self.secret.pubkey())
    }

    fn try_sign_message(
        &self,
        message: &[u8],
    ) -> Result<solana_sdk::signature::Signature, solana_sdk::signer::SignerError> {
        Ok(self.secret.sign_message(message))
    }

    fn is_interactive(&self) -> bool {
        true
    }
}