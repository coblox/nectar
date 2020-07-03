//! Execute a swap.

mod alice;
mod bitcoin;
mod bob;
mod ethereum;
mod hbit;
mod herc20;

use comit::Timestamp;
use futures::future;

pub use alice::WatchOnlyAlice;
pub use bob::WalletBob;

/// Execute a Hbit<->Herc20 swap.
pub async fn hbit_herc20<A, B>(
    alice: A,
    bob: B,
    hbit_params: hbit::Params,
    herc20_params: herc20::Params,
) -> anyhow::Result<()>
where
    A: hbit::Fund + herc20::RedeemAsAlice + hbit::Refund + SafeToFund + SafeToRedeem,
    B: herc20::Deploy + herc20::Fund + hbit::RedeemAsBob + herc20::Refund + SafeToFund,
{
    if !alice.is_safe_to_fund(herc20_params.expiry).await? {
        return Ok(());
    }

    let hbit_funded = alice.fund(&hbit_params).await?;

    if !bob.is_safe_to_fund(herc20_params.expiry).await? {
        alice.refund(&hbit_params, hbit_funded).await?;

        return Ok(());
    }

    let herc20_deployed = bob.deploy(&herc20_params).await?;

    if !bob.is_safe_to_fund(herc20_params.expiry).await? {
        alice.refund(&hbit_params, hbit_funded).await?;

        return Ok(());
    }

    let _herc20_funded = bob
        .fund(herc20_params.clone(), herc20_deployed.clone())
        .await?;

    if !alice.is_safe_to_redeem(herc20_params.expiry).await? {
        alice.refund(&hbit_params, hbit_funded).await?;
        bob.refund(&herc20_params, herc20_deployed.clone()).await?;

        return Ok(());
    }

    let herc20_redeemed = alice.redeem(&herc20_params, herc20_deployed).await?;

    let hbit_redeem = bob.redeem(&hbit_params, hbit_funded, herc20_redeemed.secret);
    let hbit_refund = alice.refund(&hbit_params, hbit_funded);

    // It's always safe for Bob to redeem, he just has to do it before
    // Alice refunds
    match future::try_select(hbit_redeem, hbit_refund).await {
        Ok(future::Either::Left((_hbit_redeemed, _))) => Ok(()),
        Ok(future::Either::Right((_hbit_refunded, _))) => Ok(()),
        Err(either) => {
            let (error, _other_future) = either.factor_first();
            Err(error)
        }
    }
}

/// Determine whether funding a smart contract is safe.
///
/// Implementations should decide based on blockchain time and
/// Beta expiry, the shorter of the two expiries.
#[async_trait::async_trait]
pub trait SafeToFund {
    async fn is_safe_to_fund(&self, beta_expiry: Timestamp) -> anyhow::Result<bool>;
}

/// Determine whether redeeming a smart contract is safe.
///
/// Implementations should decide based on blockchain time and
/// expiries.
#[async_trait::async_trait]
pub trait SafeToRedeem {
    async fn is_safe_to_redeem(&self, beta_expiry: Timestamp) -> anyhow::Result<bool>;
}

#[cfg(all(test, feature = "test-docker"))]
mod tests {
    use super::*;
    use crate::{
        bitcoin_wallet, ethereum_wallet,
        swap::{alice::wallet_actor::WalletAlice, bitcoin, bob::watch_only_actor::WatchOnlyBob},
        test_harness, Seed,
    };
    use ::bitcoin::secp256k1;
    use chrono::Utc;
    use comit::{
        asset::{
            self,
            ethereum::{Erc20Quantity, FromWei},
        },
        btsieve::{bitcoin::BitcoindConnector, ethereum::Web3Connector},
        identity, Secret, SecretHash, Timestamp,
    };
    use std::{str::FromStr, sync::Arc};
    use testcontainers::clients;

    fn hbit_params(
        secret_hash: SecretHash,
        network: ::bitcoin::Network,
        final_refund_identity: ::bitcoin::Address,
        final_redeem_identity: ::bitcoin::Address,
    ) -> (
        hbit::Params,
        hbit::PrivateDetailsFunder,
        hbit::PrivateDetailsRedeemer,
    ) {
        let asset = asset::Bitcoin::from_sat(100_000_000);
        let expiry = Timestamp::now().plus(60 * 60);

        let (private_details_funder, transient_refund_pk) = {
            let transient_refund_sk = secp256k1::SecretKey::from_str(
                "01010101010101010001020304050607ffff0000ffff00006363636363636363",
            )
            .unwrap();
            let private_details_funder = hbit::PrivateDetailsFunder {
                transient_refund_sk,
                final_refund_identity,
            };

            let transient_refund_pk =
                identity::Bitcoin::from_secret_key(&crate::SECP, &transient_refund_sk);

            (private_details_funder, transient_refund_pk)
        };

        let (private_details_redeemer, transient_redeem_pk) = {
            let transient_redeem_sk = secp256k1::SecretKey::from_str(
                "01010101010101010001020304050607ffff0000ffff00006363636363636363",
            )
            .unwrap();
            let private_details_redeemer = hbit::PrivateDetailsRedeemer {
                transient_redeem_sk,
                final_redeem_identity,
            };

            let transient_redeem_pk =
                identity::Bitcoin::from_secret_key(&crate::SECP, &transient_redeem_sk);

            (private_details_redeemer, transient_redeem_pk)
        };

        let params = hbit::Params {
            network,
            asset,
            redeem_identity: transient_redeem_pk,
            refund_identity: transient_refund_pk,
            expiry,
            secret_hash,
        };

        (params, private_details_funder, private_details_redeemer)
    }

    fn herc20_params(
        secret_hash: SecretHash,
        chain_id: ethereum::ChainId,
        redeem_identity: identity::Ethereum,
        refund_identity: identity::Ethereum,
        token_contract: ethereum::Address,
    ) -> herc20::Params {
        let quantity = Erc20Quantity::from_wei(1_000_000_000u64);
        let asset = asset::Erc20::new(token_contract, quantity);
        let expiry = Timestamp::now().plus(60 * 60);

        herc20::Params {
            asset,
            redeem_identity,
            refund_identity,
            expiry,
            chain_id,
            secret_hash,
        }
    }

    fn secret() -> Secret {
        let bytes = b"hello world, you are beautiful!!";
        Secret::from(*bytes)
    }

    #[tokio::test]
    async fn execute_alice_hbit_herc20_swap() -> anyhow::Result<()> {
        let client = clients::Cli::default();

        let bitcoin_network = ::bitcoin::Network::Regtest;
        let (bitcoin_connector, bitcoind_url, bitcoin_blockchain) = {
            let blockchain = test_harness::bitcoin::Blockchain::new(&client)?;
            blockchain.init().await?;

            let node_url = blockchain.node_url.clone();

            (
                Arc::new(BitcoindConnector::new(
                    node_url.clone(),
                    ::bitcoin::Network::Regtest,
                )?),
                node_url,
                blockchain,
            )
        };
        let ethereum_chain_id = ethereum::ChainId::regtest();
        let (ethereum_connector, ethereum_node_url, ethereum_blockchain, token_contract) = {
            let mut blockchain = test_harness::ethereum::Blockchain::new(&client)?;
            blockchain.init().await?;

            let node_url = blockchain.node_url.clone();

            let token_contract = blockchain.token_contract()?;

            (
                Arc::new(Web3Connector::new(node_url.clone())),
                node_url,
                blockchain,
                token_contract,
            )
        };

        let (alice_bitcoin_wallet, alice_ethereum_wallet) = {
            let seed = Seed::default();
            let bitcoin_wallet = {
                let wallet =
                    bitcoin_wallet::Wallet::new(seed, bitcoind_url.clone(), bitcoin_network)?;
                wallet.init().await?;

                bitcoin_blockchain
                    .mint(
                        wallet.new_address().await?,
                        asset::Bitcoin::from_sat(1_000_000_000).into(),
                    )
                    .await?;

                wallet
            };
            let ethereum_wallet = ethereum_wallet::Wallet::new(seed, ethereum_node_url.clone(), token_contract)?;

            (
                bitcoin::Wallet {
                    inner: bitcoin_wallet,
                    connector: Arc::clone(&bitcoin_connector),
                },
                ethereum::Wallet {
                    inner: ethereum_wallet,
                    connector: Arc::clone(&ethereum_connector),
                },
            )
        };

        let (bob_bitcoin_wallet, bob_ethereum_wallet) = {
            let seed = Seed::default();
            let bitcoin_wallet = {
                let wallet =
                    bitcoin_wallet::Wallet::new(seed, bitcoind_url.clone(), bitcoin_network)?;
                wallet.init().await?;

                wallet
            };
            let ethereum_wallet = ethereum_wallet::Wallet::new(seed, ethereum_node_url, token_contract)?;

            ethereum_blockchain
                .mint(
                    ethereum_wallet.account(),
                    asset::Erc20::new(token_contract, Erc20Quantity::from_wei(5_000_000_000u64)),
                    ethereum_chain_id,
                )
                .await?;

            (
                bitcoin::Wallet {
                    inner: bitcoin_wallet,
                    connector: Arc::clone(&bitcoin_connector),
                },
                ethereum::Wallet {
                    inner: ethereum_wallet,
                    connector: Arc::clone(&ethereum_connector),
                },
            )
        };

        let secret = secret();
        let secret_hash = SecretHash::new(secret);

        let start_of_swap = Utc::now().naive_local();

        let (hbit_params, private_details_funder, private_details_redeemer) = {
            let redeem_address = bob_bitcoin_wallet.inner.new_address().await?;
            let refund_address = alice_bitcoin_wallet.inner.new_address().await?;

            hbit_params(secret_hash, bitcoin_network, refund_address, redeem_address)
        };

        let herc20_params = herc20_params(
            secret_hash,
            ethereum_chain_id,
            alice_ethereum_wallet.inner.account(),
            bob_ethereum_wallet.inner.account(),
            token_contract,
        );

        let alice_swap = {
            let alice = WalletAlice {
                alpha_wallet: alice_bitcoin_wallet.clone(),
                beta_wallet: alice_ethereum_wallet.clone(),
                private_protocol_details: private_details_funder,
                secret,
                start_of_swap,
            };
            let bob = WatchOnlyBob {
                alpha_connector: Arc::clone(&bitcoin_connector),
                beta_connector: Arc::clone(&ethereum_connector),
                secret_hash,
                start_of_swap,
            };

            hbit_herc20(alice, bob, hbit_params, herc20_params.clone())
        };

        let bob_swap = {
            let alice = WatchOnlyAlice {
                alpha_connector: Arc::clone(&bitcoin_connector),
                beta_connector: Arc::clone(&ethereum_connector),
                secret_hash,
                start_of_swap,
            };
            let bob = WalletBob {
                alpha_wallet: bob_bitcoin_wallet.clone(),
                beta_wallet: bob_ethereum_wallet.clone(),
                secret_hash,
                private_protocol_details: private_details_redeemer,
                start_of_swap,
            };

            hbit_herc20(alice, bob, hbit_params, herc20_params.clone())
        };

        let alice_bitcoin_starting_balance = alice_bitcoin_wallet.inner.balance().await?;
        let bob_bitcoin_starting_balance = bob_bitcoin_wallet.inner.balance().await?;

        let alice_erc20_starting_balance = alice_ethereum_wallet
            .inner
            .erc20_balance(token_contract)
            .await?;
        let bob_erc20_starting_balance = bob_ethereum_wallet
            .inner
            .erc20_balance(token_contract)
            .await?;

        futures::future::try_join(alice_swap, bob_swap).await?;

        let alice_bitcoin_final_balance = alice_bitcoin_wallet.inner.balance().await?;
        let bob_bitcoin_final_balance = bob_bitcoin_wallet.inner.balance().await?;
        let bitcoin_max_fee = bitcoin::Amount::from_sat(100000);

        let alice_erc20_final_balance = alice_ethereum_wallet
            .inner
            .erc20_balance(token_contract)
            .await?;
        let bob_erc20_final_balance = bob_ethereum_wallet
            .inner
            .erc20_balance(token_contract)
            .await?;

        assert!(
            alice_bitcoin_final_balance
                >= alice_bitcoin_starting_balance - hbit_params.asset.into() - bitcoin_max_fee
        );
        assert!(
            bob_bitcoin_final_balance
                >= bob_bitcoin_starting_balance + hbit_params.asset.into() - bitcoin_max_fee
        );

        assert_eq!(
            alice_erc20_final_balance.quantity.to_u256(),
            alice_erc20_starting_balance.quantity.to_u256()
                + herc20_params.asset.quantity.to_u256()
        );
        assert_eq!(
            bob_erc20_final_balance.quantity.to_u256(),
            bob_erc20_starting_balance.quantity.to_u256() - herc20_params.asset.quantity.to_u256()
        );

        Ok(())
    }
}
