use crate::{
    swap::{
        db::{Database, Load, Save},
        herc20,
    },
    SwapId,
};
use anyhow::{anyhow, Context};
use comit::{
    asset::Erc20,
    ethereum::{self, Hash, Transaction, U256},
    Secret,
};
use serde::{Deserialize, Serialize};
use serde_hex::{SerHexSeq, StrictPfx};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Herc20Deployed {
    pub transaction: EthereumTransaction,
    pub location: comit::htlc_location::Ethereum,
}

impl From<Herc20Deployed> for herc20::Deployed {
    fn from(event: Herc20Deployed) -> Self {
        herc20::Deployed {
            transaction: event.transaction.into(),
            location: event.location,
        }
    }
}

impl From<herc20::Deployed> for Herc20Deployed {
    fn from(event: herc20::Deployed) -> Self {
        Herc20Deployed {
            transaction: event.transaction.into(),
            location: event.location,
        }
    }
}

impl Save<herc20::Deployed> for Database {
    fn save(&self, event: herc20::Deployed, swap_id: SwapId) -> anyhow::Result<()> {
        let stored_swap = self.get(&swap_id)?;

        match stored_swap.herc20_deployed {
            Some(_) => Err(anyhow!("Herc20 Deployed event is already stored")),
            None => {
                let mut swap = stored_swap.clone();
                swap.herc20_deployed = Some(event.into());

                let old_value = serde_json::to_vec(&stored_swap)
                    .context("Could not serialize old swap value")?;
                let new_value =
                    serde_json::to_vec(&swap).context("Could not serialize new swap value")?;

                self.db
                    .compare_and_swap(swap_id.as_bytes(), Some(old_value), Some(new_value))
                    .context("Could not write in the DB")?
                    .context("Stored swap somehow changed, aborting saving")
            }
        }
    }
}

impl Load<herc20::Deployed> for Database {
    fn load(&self, swap_id: SwapId) -> anyhow::Result<Option<herc20::Deployed>> {
        let swap = self.get(&swap_id)?;

        Ok(swap.herc20_deployed.map(Into::into))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Herc20Funded {
    pub transaction: EthereumTransaction,
    pub asset: Erc20Asset,
}

impl From<Herc20Funded> for herc20::Funded {
    fn from(event: Herc20Funded) -> Self {
        herc20::Funded {
            transaction: event.transaction.into(),
            asset: event.asset.into(),
        }
    }
}

impl From<herc20::Funded> for Herc20Funded {
    fn from(event: herc20::Funded) -> Self {
        Herc20Funded {
            transaction: event.transaction.into(),
            asset: event.asset.into(),
        }
    }
}

impl Save<herc20::Funded> for Database {
    fn save(&self, event: herc20::Funded, swap_id: SwapId) -> anyhow::Result<()> {
        let stored_swap = self.get(&swap_id)?;

        match stored_swap.herc20_funded {
            Some(_) => Err(anyhow!("Herc20 Funded event is already stored")),
            None => {
                let mut swap = stored_swap.clone();
                swap.herc20_funded = Some(event.into());

                let old_value = serde_json::to_vec(&stored_swap)
                    .context("Could not serialize old swap value")?;
                let new_value =
                    serde_json::to_vec(&swap).context("Could not serialize new swap value")?;

                self.db
                    .compare_and_swap(swap_id.as_bytes(), Some(old_value), Some(new_value))
                    .context("Could not write in the DB")?
                    .context("Stored swap somehow changed, aborting saving")
            }
        }
    }
}

impl Load<herc20::Funded> for Database {
    fn load(&self, swap_id: SwapId) -> anyhow::Result<Option<herc20::Funded>> {
        let swap = self.get(&swap_id)?;

        Ok(swap.herc20_funded.map(Into::into))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Herc20Redeemed {
    pub transaction: EthereumTransaction,
    pub secret: Secret,
}

impl From<Herc20Redeemed> for herc20::Redeemed {
    fn from(event: Herc20Redeemed) -> Self {
        herc20::Redeemed {
            transaction: event.transaction.into(),
            secret: event.secret,
        }
    }
}

impl From<herc20::Redeemed> for Herc20Redeemed {
    fn from(event: herc20::Redeemed) -> Self {
        Herc20Redeemed {
            transaction: event.transaction.into(),
            secret: event.secret,
        }
    }
}

impl Save<herc20::Redeemed> for Database {
    fn save(&self, event: herc20::Redeemed, swap_id: SwapId) -> anyhow::Result<()> {
        let stored_swap = self.get(&swap_id)?;

        match stored_swap.herc20_redeemed {
            Some(_) => Err(anyhow!("Herc20 Redeemed event is already stored")),
            None => {
                let mut swap = stored_swap.clone();
                swap.herc20_redeemed = Some(event.into());

                let old_value = serde_json::to_vec(&stored_swap)
                    .context("Could not serialize old swap value")?;
                let new_value =
                    serde_json::to_vec(&swap).context("Could not serialize new swap value")?;

                self.db
                    .compare_and_swap(swap_id.as_bytes(), Some(old_value), Some(new_value))
                    .context("Could not write in the DB")?
                    .context("Stored swap somehow changed, aborting saving")
            }
        }
    }
}

impl Load<herc20::Redeemed> for Database {
    fn load(&self, swap_id: SwapId) -> anyhow::Result<Option<herc20::Redeemed>> {
        let swap = self.get(&swap_id)?;

        Ok(swap.herc20_redeemed.map(Into::into))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Herc20Refunded {
    pub transaction: EthereumTransaction,
}

impl From<Herc20Refunded> for herc20::Refunded {
    fn from(event: Herc20Refunded) -> Self {
        herc20::Refunded {
            transaction: event.transaction.into(),
        }
    }
}

impl From<herc20::Refunded> for Herc20Refunded {
    fn from(event: herc20::Refunded) -> Self {
        Herc20Refunded {
            transaction: event.transaction.into(),
        }
    }
}

impl Save<herc20::Refunded> for Database {
    fn save(&self, event: herc20::Refunded, swap_id: SwapId) -> anyhow::Result<()> {
        let stored_swap = self.get(&swap_id)?;

        match stored_swap.herc20_refunded {
            Some(_) => Err(anyhow!("Herc20 Refunded event is already stored")),
            None => {
                let mut swap = stored_swap.clone();
                swap.herc20_refunded = Some(event.into());

                let old_value = serde_json::to_vec(&stored_swap)
                    .context("Could not serialize old swap value")?;
                let new_value =
                    serde_json::to_vec(&swap).context("Could not serialize new swap value")?;

                self.db
                    .compare_and_swap(swap_id.as_bytes(), Some(old_value), Some(new_value))
                    .context("Could not write in the DB")?
                    .context("Stored swap somehow changed, aborting saving")
            }
        }
    }
}

impl Load<herc20::Refunded> for Database {
    fn load(&self, swap_id: SwapId) -> anyhow::Result<Option<herc20::Refunded>> {
        let swap = self.get(&swap_id)?;

        Ok(swap.herc20_refunded.map(Into::into))
    }
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
pub struct EthereumTransaction {
    pub hash: Hash,
    pub to: Option<ethereum::Address>,
    pub value: U256,
    #[serde(with = "SerHexSeq::<StrictPfx>")]
    pub input: Vec<u8>,
}

impl From<EthereumTransaction> for ethereum::Transaction {
    fn from(transaction: EthereumTransaction) -> Self {
        ethereum::Transaction {
            hash: transaction.hash,
            to: transaction.to,
            value: transaction.value,
            input: transaction.input,
        }
    }
}

impl From<ethereum::Transaction> for EthereumTransaction {
    fn from(transaction: Transaction) -> Self {
        EthereumTransaction {
            hash: transaction.hash,
            to: transaction.to,
            value: transaction.value,
            input: transaction.input,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Erc20Asset {
    pub token_contract: ethereum::Address,
    pub quantity: comit::asset::Erc20Quantity,
}

impl From<Erc20Asset> for comit::asset::Erc20 {
    fn from(asset: Erc20Asset) -> Self {
        comit::asset::Erc20 {
            token_contract: asset.token_contract,
            quantity: asset.quantity,
        }
    }
}

impl From<comit::asset::Erc20> for Erc20Asset {
    fn from(asset: Erc20) -> Self {
        Erc20Asset {
            token_contract: asset.token_contract,
            quantity: asset.quantity,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swap::db::Swap;

    #[test]
    fn save_and_load_herc20_deployed() {
        let db = Database::new_test().unwrap();
        let swap = Swap::default();
        let swap_id = SwapId::default();
        let transaction = comit::transaction::Ethereum::default();
        let location = comit::htlc_location::Ethereum::random();

        db._insert(&swap_id, &swap).unwrap();

        let event = herc20::Deployed {
            transaction: transaction.clone(),
            location,
        };
        db.save(event, swap_id).unwrap();

        let stored_event: herc20::Deployed = db
            .load(swap_id)
            .expect("No error loading")
            .expect("found the event");

        assert_eq!(stored_event.transaction, transaction);
        assert_eq!(stored_event.location, location);
    }

    #[test]
    fn save_and_load_herc20_funded() {
        let db = Database::new_test().unwrap();
        let swap = Swap::default();
        let swap_id = SwapId::default();
        let transaction = comit::transaction::Ethereum::default();
        let asset = comit::asset::Erc20::new(
            ethereum::Address::random(),
            comit::asset::Erc20Quantity::from_wei_dec_str("123456789012345678").unwrap(),
        );

        db._insert(&swap_id, &swap).unwrap();

        let event = herc20::Funded {
            transaction: transaction.clone(),
            asset: asset.clone(),
        };
        db.save(event, swap_id).unwrap();

        let stored_event: herc20::Funded = db
            .load(swap_id)
            .expect("No error loading")
            .expect("found the event");

        assert_eq!(stored_event.transaction, transaction);
        assert_eq!(stored_event.asset, asset);
    }

    #[test]
    fn save_and_load_herc20_redeemed() {
        let db = Database::new_test().unwrap();
        let swap = Swap::default();
        let swap_id = SwapId::default();
        let transaction = comit::transaction::Ethereum::default();
        let secret = Secret::from_vec(b"are those thirty-two bytes? Hum.").unwrap();

        db._insert(&swap_id, &swap).unwrap();

        let event = herc20::Redeemed {
            transaction: transaction.clone(),
            secret,
        };
        db.save(event, swap_id).unwrap();

        let stored_event: herc20::Redeemed = db
            .load(swap_id)
            .expect("No error loading")
            .expect("found the event");

        assert_eq!(stored_event.transaction, transaction);
        assert_eq!(stored_event.secret, secret);
    }

    #[test]
    fn save_and_load_herc20_refunded() {
        let db = Database::new_test().unwrap();
        let swap = Swap::default();
        let swap_id = SwapId::default();
        let transaction = comit::transaction::Ethereum::default();

        db._insert(&swap_id, &swap).unwrap();

        let event = herc20::Refunded {
            transaction: transaction.clone(),
        };
        db.save(event, swap_id).unwrap();

        let stored_event: herc20::Refunded = db
            .load(swap_id)
            .expect("No error loading")
            .expect("found the event");

        assert_eq!(stored_event.transaction, transaction);
    }
}
