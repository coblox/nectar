#![allow(unreachable_code, unused_variables, clippy::unit_arg)]
#![recursion_limit = "256"]

use anyhow::Context;
use futures::{
    channel::mpsc::{Receiver, Sender},
    Future, FutureExt, SinkExt, StreamExt,
};
use futures_timer::Delay;
use nectar::{
    bitcoin, bitcoin_wallet, config,
    config::{settings, Settings},
    dai, ethereum_wallet,
    maker::{Free, PublishOrders, TakeRequestDecision},
    mid_market_rate::get_btc_dai_mid_market_rate,
    network::{self, Nectar, Orderbook, Taker},
    options::{self, Options},
    order::Position,
    swap::{self, hbit, herc20, Database, SwapKind},
    Maker, MidMarketRate, Spread, SwapId,
};
use std::{sync::Arc, time::Duration};
use structopt::StructOpt;

const ENSURED_CONSUME_ZERO_BUFFER: usize = 0;

async fn init_maker(
    bitcoin_wallet: bitcoin_wallet::Wallet,
    ethereum_wallet: ethereum_wallet::Wallet,
    maker_settings: settings::Maker,
) -> Maker {
    let initial_btc_balance = bitcoin_wallet.balance().await;

    let initial_dai_balance = ethereum_wallet.dai_balance().await;

    let btc_max_sell = maker_settings.max_sell.bitcoin;
    let dai_max_sell = maker_settings.max_sell.dai;
    let btc_fee_reserve = maker_settings.maximum_possible_fee.bitcoin;

    let initial_rate = get_btc_dai_mid_market_rate().await;

    let spread: Spread = maker_settings.spread;

    // TODO: This match is weird. If the settings does not give you want you want then it should fail earlier.
    match (initial_btc_balance, initial_dai_balance, initial_rate) {
        (Ok(initial_btc_balance), Ok(initial_dai_balance), Ok(initial_rate)) => Maker::new(
            initial_btc_balance,
            initial_dai_balance.into(),
            btc_fee_reserve,
            btc_max_sell,
            dai_max_sell,
            initial_rate,
            spread,
        ),
        // TODO better error handling
        _ => panic!("Maker initialisation failed!"),
    }
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

            if let Err(e) = sender.send(rate).await {
                tracing::trace!("Error when sending rate update from sender to receiver.")
            }

            Delay::new(update_interval).await;
        }
    };

    (future, receiver)
}

fn init_bitcoin_balance_updates(
    update_interval: Duration,
    wallet: bitcoin_wallet::Wallet,
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

            if let Err(e) = sender.send(balance).await {
                tracing::trace!("Error when sending balance update from sender to receiver.")
            }

            Delay::new(update_interval).await;
        }
    };

    (future, receiver)
}

fn init_dai_balance_updates(
    update_interval: Duration,
    wallet: ethereum_wallet::Wallet,
) -> (
    impl Future<Output = comit::Never> + Send,
    Receiver<anyhow::Result<dai::Amount>>,
) {
    let (mut sender, receiver) =
        futures::channel::mpsc::channel::<anyhow::Result<dai::Amount>>(ENSURED_CONSUME_ZERO_BUFFER);

    let future = async move {
        loop {
            let balance = wallet.dai_balance().await;

            if let Err(e) = sender.send(balance.map(|balance| balance.into())).await {
                tracing::trace!("Error when sending rate balance from sender to receiver.")
            }

            Delay::new(update_interval).await;
        }
    };

    (future, receiver)
}

async fn execute_swap(sender: Sender<FinishedSwap>) -> anyhow::Result<()> {
    let swap_id = SwapId::default();
    let position: Position =
        todo!("decision what kind of what swap it is hbit->herc20 or herc20->hbit");

    let taker: Taker = todo!("Taker has to be available after execution, e.g. load from db");

    match position {
        Position::Sell => {
            let beta_params: hbit::Params = unimplemented!();

            // TODO: await hbit->herc20 swap execution

            if let Err(e) = sender
                .send(FinishedSwap::new(
                    swap_id,
                    Free::Btc(beta_params.shared.asset.into()),
                    taker,
                ))
                .await
            {
                tracing::trace!("Error when sending execution finished from sender to receiver.")
            }
        }
        Position::Buy => {
            let beta_params: herc20::Params = unimplemented!();

            // TODO: await herc20->hbit swap execution

            if let Err(e) = sender
                .send(FinishedSwap::new(
                    swap_id,
                    Free::Dai(beta_params.asset.into()),
                    taker,
                ))
                .await
            {
                tracing::trace!("Error when sending execution finished from sender to receiver.")
            }
        }
    }

    Ok(())
}

fn handle_network_event(
    network_event: network::Event,
    maker: &mut Maker,
    swarm: &mut libp2p::Swarm<Nectar>,
    sender: Sender<FinishedSwap>,
) {
    match network_event {
        network::Event::TakeRequest(order) => {
            // decide & take & reserve
            let result = maker.process_taken_order(order.clone());

            match result {
                Ok(TakeRequestDecision::GoForSwap) => {
                    swarm.orderbook.take(order.clone());

                    match maker.new_order(order.inner.position) {
                        Ok(new_order) => {
                            swarm.orderbook.publish(new_order.into());
                        }
                        Err(e) => tracing::error!("Error when trying to create new order: {}", e),
                    }
                }
                Ok(TakeRequestDecision::RateNotProfitable)
                | Ok(TakeRequestDecision::InsufficientFunds)
                | Ok(TakeRequestDecision::CannotTradeWithTaker) => {
                    swarm.orderbook.ignore(order);
                }
                Err(e) => {
                    swarm.orderbook.ignore(order);
                    tracing::error!("Processing taken order yielded error: {}", e)
                }
            }
        }
        network::Event::SwapFinalized(local_swap_id, remote_data) => {
            tokio::spawn(execute_swap(sender));
        }
    }
}

fn handle_rate_update(
    rate_update: anyhow::Result<MidMarketRate>,
    maker: &mut Maker,
    swarm: &mut libp2p::Swarm<Nectar>,
) {
    match rate_update {
        Ok(new_rate) => {
            let reaction = maker.update_rate(new_rate);
            match reaction {
                Ok(Some(PublishOrders {
                    new_sell_order,
                    new_buy_order,
                })) => {
                    swarm.orderbook.publish(new_sell_order.into());
                    swarm.orderbook.publish(new_buy_order.into());
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
    swarm: &mut libp2p::Swarm<Nectar>,
) {
    unimplemented!()
}

fn handle_dai_balance_update(
    dai_balance_update: anyhow::Result<dai::Amount>,
    maker: &mut Maker,
    swarm: &mut libp2p::Swarm<Nectar>,
) {
    unimplemented!()
}

// TODO: I don't think `finished_swap` should be an Option
fn handle_finished_swap(finished_swap: Option<FinishedSwap>, maker: &mut Maker, db: &Database) {
    if let Some(finished_swap) = finished_swap {
        maker.process_finished_swap(finished_swap.funds_to_free, finished_swap.taker);

        let res = db.delete(&finished_swap.swap_id);
        if let Err(e) = res {
            tracing::error!(
                "Unable to fetch latest rate! Fetching rate yielded error: {}",
                e
            );
        }
    }
}

struct FinishedSwap {
    swap_id: SwapId,
    funds_to_free: Free,
    taker: Taker,
}

impl FinishedSwap {
    pub fn new(swap_id: SwapId, funds_to_free: Free, taker: Taker) -> Self {
        Self {
            swap_id,
            funds_to_free,
            taker,
        }
    }
}

#[tokio::main]
async fn main() {
    let options = options::Options::from_args();

    let settings = read_config(&options)
        .and_then(Settings::from_config_file_and_defaults)
        .expect("Could not initialize configuration");

    let dai_contract_addr: comit::ethereum::Address = settings.ethereum.dai_contract_address;

    // TODO: Proper wallet initialisation from config
    let bitcoin_wallet = bitcoin_wallet::Wallet::new(
        unimplemented!(),
        settings.bitcoin.bitcoind.node_url,
        settings.bitcoin.network,
    )
    .unwrap();
    let ethereum_wallet =
        ethereum_wallet::Wallet::new(unimplemented!(), settings.ethereum.node_url).unwrap();

    let maker = init_maker(bitcoin_wallet, ethereum_wallet, settings.maker).await;

    let orderbook = Orderbook;
    let nectar = Nectar::new(orderbook);

    let mut swarm: libp2p::Swarm<Nectar> = unimplemented!();

    let initial_sell_order = maker.new_sell_order();
    let initial_buy_order = maker.new_buy_order();

    match (initial_sell_order, initial_buy_order) {
        (Ok(sell_order), Ok(buy_order)) => {
            swarm.orderbook.publish(sell_order.into());
            swarm.orderbook.publish(buy_order.into());
        }
        _ => panic!("Unable to publish initial orders!"),
    }

    let update_interval = Duration::from_secs(15u64);

    let (rate_future, rate_update_receiver) = init_rate_updates(update_interval);
    let (btc_balance_future, btc_balance_update_receiver) =
        init_bitcoin_balance_updates(update_interval, bitcoin_wallet);
    let (dai_balance_future, dai_balance_update_receiver) =
        init_dai_balance_updates(update_interval, ethereum_wallet);

    tokio::spawn(rate_future);
    tokio::spawn(btc_balance_future);
    tokio::spawn(dai_balance_future);

    let (swap_execution_finished_sender, swap_execution_finished_receiver) =
        futures::channel::mpsc::channel::<FinishedSwap>(ENSURED_CONSUME_ZERO_BUFFER);

    let db = Arc::new(Database::new(todo!(
        "try to load from config, otherwise default?"
    )))
    .unwrap();

    todo!("tokio::spawn(respawn_swaps())");

    loop {
        futures::select! {
            // TODO: I don't think we need to handle the Option
            finished_swap = swap_execution_finished_receiver.next().fuse() => {
                handle_finished_swap(finished_swap, &mut maker, &db);
            },
            network_event = swarm.next().fuse() => {
                handle_network_event(network_event, &mut maker, &mut swarm, swap_execution_finished_sender.clone());
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

#[allow(dead_code)]
fn respawn_swaps(
    db: Arc<Database>,
    bitcoin_wallet: Arc<bitcoin_wallet::Wallet>,
    ethereum_wallet: Arc<ethereum_wallet::Wallet>,
    bitcoin_connector: Arc<comit::btsieve::bitcoin::BitcoindConnector>,
    ethereum_connector: Arc<comit::btsieve::ethereum::Web3Connector>,
    swap_execution_finished_sender: Sender<FinishedSwap>,
) -> anyhow::Result<()> {
    for swap in db.load_all()?.into_iter() {
        match swap {
            SwapKind::HbitHerc20(swap) => {
                tokio::spawn(swap::nectar_hbit_herc20(
                    Arc::clone(&db),
                    Arc::clone(&bitcoin_wallet),
                    Arc::clone(&ethereum_wallet),
                    Arc::clone(&bitcoin_connector),
                    Arc::clone(&ethereum_connector),
                    swap,
                ));
            }
            SwapKind::Herc20Hbit(_) => todo!(),
        }
    }

    Ok(())
}

fn read_config(options: &Options) -> anyhow::Result<config::File> {
    // if the user specifies a config path, use it
    if let Some(path) = &options.config_file {
        eprintln!("Using config file {}", path.display());

        return config::File::read(&path)
            .with_context(|| format!("failed to read config file {}", path.display()));
    }

    // try to load default config
    let default_path = nectar::default_config_path()?;

    if !default_path.exists() {
        return Ok(config::File::default());
    }

    eprintln!(
        "Using config file at default path: {}",
        default_path.display()
    );

    config::File::read(&default_path)
        .with_context(|| format!("failed to read config file {}", default_path.display()))
}
