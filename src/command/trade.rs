use crate::{
    bitcoin,
    command::{into_history_trade, FinishedSwap},
    config::Settings,
    ethereum::{self, dai},
    history::History,
    maker::PublishOrders,
    mid_market_rate::get_btc_dai_mid_market_rate,
    network::{self, Swarm},
    swap::{Database, SwapKind, SwapParams},
    Maker, MidMarketRate, Seed, Spread,
};
use anyhow::Context;
use comit::btsieve::{bitcoin::BitcoindConnector, ethereum::Web3Connector};
use futures::{
    channel::mpsc::{Receiver, Sender},
    Future, FutureExt, SinkExt, StreamExt, TryFutureExt,
};
use futures_timer::Delay;

use crate::{
    maker::TakeRequestDecision,
    network::{new_swarm, ActivePeer, SetupSwapContext},
};
use comit::{Position, Role};
use std::{sync::Arc, time::Duration};

const ENSURED_CONSUME_ZERO_BUFFER: usize = 0;

pub async fn trade(
    seed: &Seed,
    settings: Settings,
    bitcoin_wallet: bitcoin::Wallet,
    ethereum_wallet: ethereum::Wallet,
) -> anyhow::Result<()> {
    let bitcoin_wallet = Arc::new(bitcoin_wallet);
    let ethereum_wallet = Arc::new(ethereum_wallet);

    let mut maker = init_maker(
        Arc::clone(&bitcoin_wallet),
        Arc::clone(&ethereum_wallet),
        settings.clone(),
    )
    .await
    .context("Could not initialise Maker")?;

    #[cfg(not(test))]
    let db = Arc::new(Database::new(&settings.data.dir.join("database"))?);
    #[cfg(test)]
    let db = Arc::new(Database::new_test()?);

    let mut swarm = new_swarm(
        network::Seed::new(seed.bytes()),
        &settings,
        Arc::clone(&bitcoin_wallet),
        Arc::clone(&ethereum_wallet),
        Arc::clone(&db),
    )?;

    let initial_sell_order = maker
        .new_sell_order()
        .context("Could not generate sell order")?;

    let initial_buy_order = maker
        .new_buy_order()
        .context("Could not generate buy order")?;

    swarm
        .orderbook
        .publish(initial_sell_order.to_comit_order(maker.swap_protocol(Position::Buy)));
    swarm
        .orderbook
        .publish(initial_buy_order.to_comit_order(maker.swap_protocol(Position::Sell)));

    let update_interval = Duration::from_secs(15u64);

    let (rate_future, mut rate_update_receiver) = init_rate_updates(update_interval);
    let (btc_balance_future, mut btc_balance_update_receiver) =
        init_bitcoin_balance_updates(update_interval, Arc::clone(&bitcoin_wallet));
    let (dai_balance_future, mut dai_balance_update_receiver) =
        init_dai_balance_updates(update_interval, Arc::clone(&ethereum_wallet));

    tokio::spawn(rate_future);
    tokio::spawn(btc_balance_future);
    tokio::spawn(dai_balance_future);

    let (swap_execution_finished_sender, mut swap_execution_finished_receiver) =
        futures::channel::mpsc::channel::<FinishedSwap>(ENSURED_CONSUME_ZERO_BUFFER);

    let mut history = History::new(settings.data.dir.join("history.csv").as_path())?;

    let bitcoin_connector = Arc::new(BitcoindConnector::new(settings.bitcoin.bitcoind.node_url)?);
    let ethereum_connector = Arc::new(Web3Connector::new(settings.ethereum.node_url));

    respawn_swaps(
        Arc::clone(&db),
        &mut maker,
        Arc::clone(&bitcoin_wallet),
        Arc::clone(&ethereum_wallet),
        Arc::clone(&bitcoin_connector),
        Arc::clone(&ethereum_connector),
        swap_execution_finished_sender.clone(),
    )
    .context("Could not respawn swaps")?;

    loop {
        futures::select! {
            finished_swap = swap_execution_finished_receiver.next().fuse() => {
                if let Some(finished_swap) = finished_swap {
                    handle_finished_swap(finished_swap, &mut maker, &db, &mut history, &mut swarm).await;
                }
            },
            network_event = swarm.next().fuse() => {
                handle_network_event(
                    network_event,
                    &mut maker,
                    &mut swarm,
                    Arc::clone(&db),
                    Arc::clone(&bitcoin_wallet),
                    Arc::clone(&ethereum_wallet),
                    Arc::clone(&bitcoin_connector),
                    Arc::clone(&ethereum_connector),
                    swap_execution_finished_sender.clone(),
                ).await;
            },
            rate_update = rate_update_receiver.next().fuse() => {
                handle_rate_update(rate_update.unwrap(), &mut maker, &mut swarm);
            },
            btc_balance_update = btc_balance_update_receiver.next().fuse() => {
                handle_btc_balance_update(btc_balance_update.unwrap(), &mut maker, &mut swarm);
            },
            dai_balance_update = dai_balance_update_receiver.next().fuse() => {
                handle_dai_balance_update(dai_balance_update.unwrap(), &mut maker, &mut swarm);
            }
        }
    }
}

async fn init_maker(
    bitcoin_wallet: Arc<bitcoin::Wallet>,
    ethereum_wallet: Arc<ethereum::Wallet>,
    settings: Settings,
) -> anyhow::Result<Maker> {
    let initial_btc_balance = bitcoin_wallet
        .balance()
        .await
        .context("Could not get Bitcoin balance")?;

    let initial_dai_balance = ethereum_wallet
        .dai_balance()
        .await
        .context("Could not get Dai balance")?;

    let btc_max_sell = settings.maker.max_sell.bitcoin;
    let dai_max_sell = settings.maker.max_sell.dai.clone();
    let btc_fee_reserve = settings.maker.maximum_possible_fee.bitcoin;

    let initial_rate = get_btc_dai_mid_market_rate()
        .await
        .context("Could not get rate")?;

    let spread: Spread = settings.maker.spread;

    Ok(Maker::new(
        initial_btc_balance,
        initial_dai_balance,
        btc_fee_reserve,
        btc_max_sell,
        dai_max_sell,
        initial_rate,
        spread,
        settings.bitcoin.network,
        settings.ethereum.chain,
        // todo: get from config
        Role::Bob,
    ))
}

fn init_rate_updates(
    update_interval: Duration,
) -> (
    impl Future<Output = comit::Never> + Send,
    Receiver<anyhow::Result<MidMarketRate>>,
) {
    let (mut sender, receiver) = futures::channel::mpsc::channel::<anyhow::Result<MidMarketRate>>(
        ENSURED_CONSUME_ZERO_BUFFER,
    );

    let future = async move {
        loop {
            let rate = get_btc_dai_mid_market_rate().await;

            let _ = sender.send(rate).await.map_err(|e| {
                tracing::trace!(
                    "Error when sending rate update from sender to receiver: {}",
                    e
                )
            });

            Delay::new(update_interval).await;
        }
    };

    (future, receiver)
}

fn init_bitcoin_balance_updates(
    update_interval: Duration,
    wallet: Arc<bitcoin::Wallet>,
) -> (
    impl Future<Output = comit::Never> + Send,
    Receiver<anyhow::Result<bitcoin::Amount>>,
) {
    let (mut sender, receiver) = futures::channel::mpsc::channel::<anyhow::Result<bitcoin::Amount>>(
        ENSURED_CONSUME_ZERO_BUFFER,
    );

    let future = async move {
        loop {
            let balance = wallet.balance().await;

            let _ = sender.send(balance).await.map_err(|e| {
                tracing::trace!(
                    "Error when sending balance update from sender to receiver: {}",
                    e
                )
            });

            Delay::new(update_interval).await;
        }
    };

    (future, receiver)
}

fn init_dai_balance_updates(
    update_interval: Duration,
    wallet: Arc<ethereum::Wallet>,
) -> (
    impl Future<Output = comit::Never> + Send,
    Receiver<anyhow::Result<dai::Amount>>,
) {
    let (mut sender, receiver) =
        futures::channel::mpsc::channel::<anyhow::Result<dai::Amount>>(ENSURED_CONSUME_ZERO_BUFFER);

    let future = async move {
        loop {
            let balance = wallet.dai_balance().await;

            let _ = sender.send(balance).await.map_err(|e| {
                tracing::trace!(
                    "Error when sending rate balance from sender to receiver: {}",
                    e
                )
            });

            Delay::new(update_interval).await;
        }
    };

    (future, receiver)
}

#[allow(clippy::too_many_arguments)]
async fn execute_swap(
    db: Arc<Database>,
    bitcoin_wallet: Arc<bitcoin::Wallet>,
    ethereum_wallet: Arc<ethereum::Wallet>,
    bitcoin_connector: Arc<comit::btsieve::bitcoin::BitcoindConnector>,
    ethereum_connector: Arc<comit::btsieve::ethereum::Web3Connector>,
    mut finished_swap_sender: Sender<FinishedSwap>,
    swap: SwapKind,
) -> anyhow::Result<()> {
    db.insert_swap(swap.clone()).await?;

    swap.execute(
        Arc::clone(&db),
        Arc::clone(&bitcoin_wallet),
        Arc::clone(&ethereum_wallet),
        Arc::clone(&bitcoin_connector),
        Arc::clone(&ethereum_connector),
    )
    .await?;

    let _ = finished_swap_sender
        .send(FinishedSwap::new(
            swap.clone(),
            swap.params().taker,
            chrono::Utc::now(),
        ))
        .await
        .map_err(|_| {
            tracing::trace!("Error when sending execution finished from sender to receiver.")
        });

    Ok(())
}

fn respawn_swaps(
    db: Arc<Database>,
    maker: &mut Maker,
    bitcoin_wallet: Arc<bitcoin::Wallet>,
    ethereum_wallet: Arc<ethereum::Wallet>,
    bitcoin_connector: Arc<comit::btsieve::bitcoin::BitcoindConnector>,
    ethereum_connector: Arc<comit::btsieve::ethereum::Web3Connector>,
    finished_swap_sender: Sender<FinishedSwap>,
) -> anyhow::Result<()> {
    for swap in db.all_swaps()?.into_iter() {
        // Reserve funds
        match swap {
            SwapKind::HbitHerc20(SwapParams {
                ref herc20_params, ..
            }) => {
                let fund_amount = herc20_params.asset.clone().into();
                maker.dai_reserved_funds = maker.dai_reserved_funds.clone() + fund_amount;
            }
            SwapKind::Herc20Hbit(SwapParams { hbit_params, .. }) => {
                let fund_amount = hbit_params.shared.asset.into();
                maker.btc_reserved_funds = maker.btc_reserved_funds + fund_amount + maker.btc_fee;
            }
        };

        tokio::spawn(execute_swap(
            Arc::clone(&db),
            Arc::clone(&bitcoin_wallet),
            Arc::clone(&ethereum_wallet),
            Arc::clone(&bitcoin_connector),
            Arc::clone(&ethereum_connector),
            finished_swap_sender.clone(),
            swap,
        ));
    }

    Ok(())
}

fn handle_rate_update(
    rate_update: anyhow::Result<MidMarketRate>,
    maker: &mut Maker,
    swarm: &mut Swarm,
) {
    match rate_update {
        Ok(new_rate) => {
            let result = maker.update_rate(new_rate);
            match result {
                Ok(Some(PublishOrders {
                    new_sell_order,
                    new_buy_order,
                })) => {
                    swarm.orderbook.publish(
                        new_sell_order.to_comit_order(maker.swap_protocol(Position::Sell)),
                    );
                    swarm
                        .orderbook
                        .publish(new_buy_order.to_comit_order(maker.swap_protocol(Position::Buy)));
                    swarm.orderbook.clear_own_orders();
                }

                Ok(None) => (),
                Err(e) => tracing::warn!("Rate update yielded error: {}", e),
            }
        }
        Err(e) => {
            maker.invalidate_rate();
            tracing::error!(
                "Unable to fetch latest rate! Fetching rate yielded error: {}",
                e
            );
        }
    }
}

fn handle_btc_balance_update(
    btc_balance_update: anyhow::Result<bitcoin::Amount>,
    maker: &mut Maker,
    swarm: &mut Swarm,
) {
    match btc_balance_update {
        Ok(btc_balance) => match maker.update_bitcoin_balance(btc_balance) {
            Ok(Some(new_sell_order)) => {
                let order = new_sell_order.to_comit_order(maker.swap_protocol(Position::Sell));
                swarm.orderbook.clear_own_orders();
                swarm.orderbook.publish(order);
            }
            Ok(None) => (),
            Err(e) => tracing::warn!("Bitcoin balance update yielded error: {}", e),
        },
        Err(e) => {
            maker.invalidate_bitcoin_balance();
            tracing::error!(
                "Unable to fetch bitcoin balance! Fetching balance yielded error: {}",
                e
            );
        }
    }
}

fn handle_dai_balance_update(
    dai_balance_update: anyhow::Result<dai::Amount>,
    maker: &mut Maker,
    swarm: &mut Swarm,
) {
    match dai_balance_update {
        Ok(dai_balance) => match maker.update_dai_balance(dai_balance) {
            Ok(Some(new_buy_order)) => {
                let order = new_buy_order.to_comit_order(maker.swap_protocol(Position::Buy));
                swarm.orderbook.clear_own_orders();
                swarm.orderbook.publish(order);
            }
            Ok(None) => (),
            Err(e) => tracing::warn!("Dai balance update yielded error: {}", e),
        },
        Err(e) => {
            maker.invalidate_dai_balance();
            tracing::error!(
                "Unable to fetch dai balance! Fetching balance yielded error: {}",
                e
            );
        }
    }
}

async fn handle_finished_swap(
    finished_swap: FinishedSwap,
    maker: &mut Maker,
    db: &Database,
    history: &mut History,
    _swarm: &mut Swarm,
) {
    {
        let trade = into_history_trade(
            finished_swap.peer.peer_id(),
            finished_swap.swap.clone(),
            #[cfg(not(test))]
            finished_swap.final_timestamp,
        );

        let _ = history.write(trade).map_err(|error| {
            tracing::error!(
                "Unable to register history entry: {}; {:?}",
                error,
                finished_swap
            )
        });
    }

    let (dai, btc, swap_id) = match finished_swap.swap {
        SwapKind::HbitHerc20(swap) => (Some(swap.herc20_params.asset.into()), None, swap.swap_id),
        SwapKind::Herc20Hbit(swap) => (
            None,
            Some(swap.hbit_params.shared.asset.into()),
            swap.swap_id,
        ),
    };

    maker.free_funds(dai, btc);

    let _ = db
        .remove_active_peer(&finished_swap.peer)
        .await
        .map_err(|error| tracing::error!("Unable to remove from active takers: {}", error));

    let _ = db
        .remove_swap(&swap_id)
        .await
        .map_err(|error| tracing::error!("Unable to delete swap from db: {}", error));
}

#[allow(clippy::too_many_arguments)]
async fn handle_network_event(
    network_event: network::Event,
    maker: &mut Maker,
    swarm: &mut Swarm,
    db: Arc<Database>,
    bitcoin_wallet: Arc<bitcoin::Wallet>,
    ethereum_wallet: Arc<ethereum::Wallet>,
    bitcoin_connector: Arc<comit::btsieve::bitcoin::BitcoindConnector>,
    ethereum_connector: Arc<comit::btsieve::ethereum::Web3Connector>,
    finished_swap_sender: Sender<FinishedSwap>,
) {
    match network_event {
        network::Event::OrderMatch {
            form,
            to,
            to_send,
            common,
            swap_protocol,
            swap_id,
            match_ref_point,
            bitcoin_transient_key_index,
        } => {
            let result = maker.process_taken_order(form);

            match result {
                Ok(TakeRequestDecision::GoForSwap) => {
                    if let Err(e) = swarm.setup_swap.send(
                        &to,
                        to_send,
                        common,
                        swap_protocol,
                        SetupSwapContext {
                            swap_id,
                            match_ref_point,
                            bitcoin_transient_key_index,
                        },
                    ) {
                        tracing::error!("Sending setup swap message yielded error: {}", e)
                    }

                    let _ = db
                        .insert_active_peer(ActivePeer { peer_id: to })
                        .await
                        .map_err(|e| tracing::error!("Failed to confirm order: {}", e));

                    // todo: publish new order here?
                    // What if i publish a new order here and the does go
                    // through?
                }
                Ok(TakeRequestDecision::InsufficientFunds) => tracing::info!("Insufficient funds"),
                Ok(TakeRequestDecision::RateNotProfitable) => tracing::info!("Rate not profitable"),
                Err(e) => tracing::error!("Processing taken order yielded error: {}", e),
            };
        }
        network::Event::SpawnSwap(swap) => {
            let swap_id = swap.swap_id();

            let res = db
                .insert_swap(swap.clone())
                .map_err(|e| tracing::error!("Could not insert swap {}: {:?}", swap_id, e))
                .await;

            if res.is_ok() {
                let _ = tokio::spawn(execute_swap(
                    Arc::clone(&db),
                    Arc::clone(&bitcoin_wallet),
                    Arc::clone(&ethereum_wallet),
                    Arc::clone(&bitcoin_connector),
                    Arc::clone(&ethereum_connector),
                    finished_swap_sender,
                    swap,
                ))
                .await
                .map_err(|e| {
                    tracing::error!("Execution failed for swap swap {}: {:?}", swap_id, e)
                });
            }
        }
    }
}

#[cfg(all(test, feature = "test-docker"))]
mod tests {
    use super::*;
    use crate::{
        config::{settings, Data, Logging, MaxSell, Network},
        swap::herc20::asset::ethereum::FromWei,
        test_harness, Seed,
    };
    use comit::{asset, asset::Erc20Quantity, ethereum::ChainId};
    use ethereum::ether;
    use log::LevelFilter;

    // Run cargo test with `--ignored --nocapture` to see the `println output`
    #[ignore]
    #[tokio::test]
    async fn trade_command() {
        let client = testcontainers::clients::Cli::default();
        let seed = Seed::random().unwrap();

        let bitcoin_blockchain = test_harness::bitcoin::Blockchain::new(&client).unwrap();
        bitcoin_blockchain.init().await.unwrap();

        let mut ethereum_blockchain = test_harness::ethereum::Blockchain::new(&client).unwrap();
        ethereum_blockchain.init().await.unwrap();

        let settings = Settings {
            maker: settings::Maker {
                max_sell: MaxSell {
                    bitcoin: None,
                    dai: None,
                },
                spread: Default::default(),
                maximum_possible_fee: Default::default(),
            },
            network: Network {
                listen: vec!["/ip4/98.97.96.95/tcp/20500"
                    .parse()
                    .expect("invalid multiaddr")],
            },
            data: Data {
                dir: Default::default(),
            },
            logging: Logging {
                level: LevelFilter::Trace,
            },
            bitcoin: Default::default(),
            ethereum: settings::Ethereum {
                node_url: ethereum_blockchain.node_url.clone(),
                chain: ethereum::Chain::new(
                    ChainId::GETH_DEV,
                    ethereum_blockchain.token_contract(),
                ),
            },
        };

        let bitcoin_wallet = bitcoin::Wallet::new(
            seed,
            bitcoin_blockchain.node_url.clone(),
            ::bitcoin::Network::Regtest,
        )
        .await
        .unwrap();

        let ethereum_wallet = crate::ethereum::Wallet::new(
            seed,
            ethereum_blockchain.node_url.clone(),
            settings.ethereum.chain,
        )
        .await
        .unwrap();

        bitcoin_blockchain
            .mint(
                bitcoin_wallet.new_address().await.unwrap(),
                asset::Bitcoin::from_sat(1_000_000_000).into(),
            )
            .await
            .unwrap();

        ethereum_blockchain
            .mint_ether(
                ethereum_wallet.account(),
                ether::Amount::from(1_000_000_000_000_000_000u64),
                settings.ethereum.chain.chain_id(),
            )
            .await
            .unwrap();
        ethereum_blockchain
            .mint_erc20_token(
                ethereum_wallet.account(),
                asset::Erc20::new(
                    settings.ethereum.chain.dai_contract_address(),
                    Erc20Quantity::from_wei(1_000_000_000_000_000_000u64),
                ),
                settings.ethereum.chain.chain_id(),
            )
            .await
            .unwrap();

        let _ = trade(&seed, settings, bitcoin_wallet, ethereum_wallet)
            .await
            .unwrap();
    }
}
