use crate::swap::hbit;
use comit::{asset, Secret};
use std::sync::Arc;

pub use crate::bitcoin::Amount;
pub use ::bitcoin::{secp256k1::SecretKey, Address, Block, BlockHash, OutPoint, Transaction};

#[derive(Debug, Clone)]
pub struct Wallet {
    pub inner: Arc<crate::bitcoin_wallet::Wallet>,
    pub connector: Arc<comit::btsieve::bitcoin::BitcoindConnector>,
}

#[async_trait::async_trait]
impl hbit::ExecuteFund for Wallet {
    async fn execute_fund(&self, params: &hbit::Params) -> anyhow::Result<hbit::Funded> {
        let action = params.shared.build_fund_action();

        let txid = self
            .inner
            .send_to_address(action.to, action.amount.into(), action.network)
            .await?;
        let transaction = self.inner.get_raw_transaction(txid).await?;

        // TODO: This code is copied straight from COMIT lib. We
        // should find a way of not having to duplicate this logic
        let location = transaction
            .output
            .iter()
            .enumerate()
            .map(|(index, txout)| {
                // Casting a usize to u32 can lead to truncation on 64bit platforms
                // However, bitcoin limits the number of inputs to u32 anyway, so this
                // is not a problem for us.
                #[allow(clippy::cast_possible_truncation)]
                (index as u32, txout)
            })
            .find(|(_, txout)| {
                txout.script_pubkey == params.shared.compute_address().script_pubkey()
            })
            .map(|(vout, _txout)| bitcoin::OutPoint { txid, vout });

        let location = location.ok_or_else(|| {
            anyhow::anyhow!("Fund transaction does not contain expected outpoint")
        })?;
        let asset = asset::Bitcoin::from_sat(transaction.output[location.vout as usize].value);

        Ok(hbit::Funded { asset, location })
    }
}

#[async_trait::async_trait]
impl hbit::ExecuteRedeem for Wallet {
    async fn execute_redeem(
        &self,
        params: hbit::Params,
        fund_event: hbit::Funded,
        secret: Secret,
    ) -> anyhow::Result<hbit::Redeemed> {
        let redeem_address = self.inner.new_address().await?;

        let action = params.shared.build_redeem_action(
            &crate::SECP,
            fund_event.asset,
            fund_event.location,
            params.transient_sk,
            redeem_address,
            secret,
        )?;
        let transaction = self.spend(action).await?;

        Ok(hbit::Redeemed {
            transaction,
            secret,
        })
    }
}

impl Wallet {
    pub async fn redeem(
        &self,
        action: hbit::BroadcastSignedTransaction,
    ) -> anyhow::Result<bitcoin::Transaction> {
        self.spend(action).await
    }

    pub async fn refund(
        &self,
        action: hbit::BroadcastSignedTransaction,
    ) -> anyhow::Result<bitcoin::Transaction> {
        self.spend(action).await
    }

    async fn spend(
        &self,
        action: hbit::BroadcastSignedTransaction,
    ) -> anyhow::Result<bitcoin::Transaction> {
        let _txid = self
            .inner
            .send_raw_transaction(action.transaction.clone(), action.network)
            .await?;

        Ok(action.transaction)
    }
}
