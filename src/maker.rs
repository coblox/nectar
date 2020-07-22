#![allow(unused_variables)]

use crate::{
    bitcoin,
    ethereum::{
        self,
        dai::{self, DaiContractAddress},
    },
    network::{self, Taker},
    order::{BtcDaiOrder, Position, Symbol},
    rate::Spread,
    MidMarketRate,
};

// TODO: Figure out why this is an enum
#[derive(Debug, PartialEq)]
pub enum NewOrder {
    Created(BtcDaiOrder),
}

#[derive(Debug, Clone)]
pub struct TakenOrder {
    pub inner: BtcDaiOrder,
    pub taker: Taker,
}

// Bundles the state of the application
#[derive(Debug)]
pub struct Maker {
    btc_balance: Option<bitcoin::Amount>,
    dai_balance: Option<dai::Amount>,
    pub btc_fee: bitcoin::Amount,
    pub btc_reserved_funds: bitcoin::Amount,
    pub dai_reserved_funds: dai::Amount,
    btc_max_sell_amount: Option<bitcoin::Amount>,
    dai_max_sell_amount: Option<dai::Amount>,
    mid_market_rate: Option<MidMarketRate>,
    spread: Spread,
    dai_contract_address: DaiContractAddress,
    bitcoin_network: bitcoin::Network,
    ethereum_network: ethereum::ChainId,
}

impl Maker {
    #![allow(clippy::too_many_arguments)]
    pub fn new(
        btc_balance: bitcoin::Amount,
        dai_balance: dai::Amount,
        btc_fee: bitcoin::Amount,
        btc_max_sell_amount: Option<bitcoin::Amount>,
        dai_max_sell_amount: Option<dai::Amount>,
        mid_market_rate: MidMarketRate,
        spread: Spread,
        dai_contract_address: DaiContractAddress,
        bitcoin_network: bitcoin::Network,
        ethereum_network: ethereum::ChainId,
    ) -> Self {
        Maker {
            btc_balance: Some(btc_balance),
            dai_balance: Some(dai_balance),
            btc_fee,
            btc_reserved_funds: Default::default(),
            dai_reserved_funds: Default::default(),
            btc_max_sell_amount,
            dai_max_sell_amount,
            mid_market_rate: Some(mid_market_rate),
            spread,
            dai_contract_address,
            bitcoin_network,
            ethereum_network,
        }
    }

    pub fn update_rate(
        &mut self,
        mid_market_rate: MidMarketRate,
    ) -> anyhow::Result<Option<PublishOrders>> {
        match self.mid_market_rate {
            Some(previous_mid_market_rate) if previous_mid_market_rate == mid_market_rate => {
                Ok(None)
            }
            _ => {
                self.mid_market_rate = Some(mid_market_rate);

                Ok(Some(PublishOrders {
                    new_sell_order: self.new_sell_order()?,
                    new_buy_order: self.new_buy_order()?,
                }))
            }
        }
    }

    pub fn invalidate_rate(&mut self) {
        self.mid_market_rate = None;
    }

    pub fn update_bitcoin_balance(
        &mut self,
        balance: bitcoin::Amount,
    ) -> anyhow::Result<Option<BtcDaiOrder>> {
        // if we had a balance and the balance did not change => no new orders
        if let Some(previous_balance) = self.btc_balance {
            if previous_balance == balance {
                return Ok(None);
            }
        }

        self.btc_balance = Some(balance);
        let order = self.new_sell_order()?;
        Ok(Some(order))
    }

    pub fn invalidate_bitcoin_balance(&mut self) {
        self.btc_balance = None;
    }

    pub fn update_dai_balance(
        &mut self,
        balance: dai::Amount,
    ) -> anyhow::Result<Option<BtcDaiOrder>> {
        // if we had a balance and the balance did not change => no new orders
        if let Some(previous_balance) = self.dai_balance.clone() {
            if previous_balance == balance {
                return Ok(None);
            }
        }

        self.dai_balance = Some(balance);
        let order = self.new_buy_order()?;
        Ok(Some(order))
    }

    pub fn invalidate_dai_balance(&mut self) {
        self.dai_balance = None;
    }

    pub fn new_sell_order(&self) -> anyhow::Result<BtcDaiOrder> {
        match (self.mid_market_rate, self.btc_balance) {
            (Some(mid_market_rate), Some(btc_balance)) => BtcDaiOrder::new_sell(
                btc_balance,
                self.btc_fee,
                self.btc_reserved_funds,
                self.btc_max_sell_amount,
                mid_market_rate.into(),
                self.spread,
                self.dai_contract_address,
                self.bitcoin_network,
                self.ethereum_network,
            ),
            (None, _) => anyhow::bail!(RateNotAvailable(Position::Sell)),
            (_, None) => anyhow::bail!(BalanceNotAvailable(Symbol::Btc)),
        }
    }

    pub fn new_buy_order(&self) -> anyhow::Result<BtcDaiOrder> {
        match (self.mid_market_rate, self.dai_balance.clone()) {
            (Some(mid_market_rate), Some(dai_balance)) => BtcDaiOrder::new_buy(
                dai_balance,
                self.dai_reserved_funds.clone(),
                self.dai_max_sell_amount.clone(),
                mid_market_rate.into(),
                self.spread,
                self.dai_contract_address,
                self.bitcoin_network,
                self.ethereum_network,
            ),
            (None, _) => anyhow::bail!(RateNotAvailable(Position::Buy)),
            (_, None) => anyhow::bail!(BalanceNotAvailable(Symbol::Dai)),
        }
    }

    pub fn new_order(&self, position: Position) -> anyhow::Result<BtcDaiOrder> {
        match position {
            Position::Buy => self.new_buy_order(),
            Position::Sell => self.new_sell_order(),
        }
    }

    /// Decide whether we should proceed with order,
    /// Confirm with the order book
    /// Re & take & reserve
    pub fn process_taken_order(
        &mut self,
        order: TakenOrder,
    ) -> anyhow::Result<TakeRequestDecision> {
        match self.mid_market_rate {
            Some(current_mid_market_rate) => {
                let current_profitable_rate = self
                    .spread
                    .apply(current_mid_market_rate.into(), order.inner.position)?;

                if !order.inner.is_as_good_as(current_mid_market_rate)? {
                    return Ok(TakeRequestDecision::RateNotProfitable);
                }

                match order.inner {
                    order
                    @
                    BtcDaiOrder {
                        position: Position::Buy,
                        ..
                    } => match self.dai_balance {
                        Some(ref dai_balance) => {
                            let updated_dai_reserved_funds =
                                self.dai_reserved_funds.clone() + order.quote.amount;
                            if updated_dai_reserved_funds > *dai_balance {
                                return Ok(TakeRequestDecision::InsufficientFunds);
                            }

                            self.dai_reserved_funds = updated_dai_reserved_funds;
                        }
                        None => anyhow::bail!(BalanceNotAvailable(Symbol::Dai)),
                    },
                    order
                    @
                    BtcDaiOrder {
                        position: Position::Sell,
                        ..
                    } => match self.btc_balance {
                        Some(btc_balance) => {
                            let updated_btc_reserved_funds =
                                self.btc_reserved_funds + order.base.amount + self.btc_fee;
                            if updated_btc_reserved_funds > btc_balance {
                                return Ok(TakeRequestDecision::InsufficientFunds);
                            }

                            self.btc_reserved_funds = updated_btc_reserved_funds;
                        }
                        None => anyhow::bail!(BalanceNotAvailable(Symbol::Btc)),
                    },
                };

                Ok(TakeRequestDecision::GoForSwap)
            }
            None => anyhow::bail!(RateNotAvailable(order.inner.position)),
        }
    }

    pub fn free_funds(&mut self, dai: Option<dai::Amount>, bitcoin: Option<bitcoin::Amount>) {
        if let Some(amount) = dai {
            self.dai_reserved_funds = self.dai_reserved_funds.clone() - amount;
        }

        if let Some(amount) = bitcoin {
            self.btc_reserved_funds = self.btc_reserved_funds - (amount + self.btc_fee);
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum TakeRequestDecision {
    GoForSwap,
    RateNotProfitable,
    InsufficientFunds,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PublishOrders {
    pub new_sell_order: BtcDaiOrder,
    pub new_buy_order: BtcDaiOrder,
}

#[derive(Debug, Copy, Clone, thiserror::Error)]
#[error("Rate not available when trying to create new {0} order.")]
pub struct RateNotAvailable(Position);

#[derive(Debug, Copy, Clone, thiserror::Error)]
#[error("{0} balance not available.")]
pub struct BalanceNotAvailable(Symbol);

impl From<&network::TakenOrder> for TakenOrder {
    fn from(from: &network::TakenOrder) -> Self {
        Self {
            inner: from.inner.clone(),
            taker: from.taker.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::Taker;
    use crate::{
        order::{BtcDaiOrder, Position},
        MidMarketRate, Rate,
    };
    use comit::ethereum::ChainId;
    use std::convert::TryFrom;

    impl Default for Maker {
        fn default() -> Self {
            Self {
                btc_balance: Some(bitcoin::Amount::default()),
                dai_balance: Some(dai::Amount::default()),
                btc_fee: bitcoin::Amount::default(),
                btc_reserved_funds: bitcoin::Amount::default(),
                dai_reserved_funds: dai::Amount::default(),
                btc_max_sell_amount: None,
                dai_max_sell_amount: None,
                mid_market_rate: Some(MidMarketRate::default()),
                spread: Spread::default(),
                dai_contract_address: DaiContractAddress::Mainnet,
                bitcoin_network: bitcoin::Network::Bitcoin,
                ethereum_network: ethereum::ChainId::mainnet(),
            }
        }
    }

    impl Default for TakenOrder {
        fn default() -> Self {
            Self {
                inner: BtcDaiOrder::default(),
                taker: Taker::default(),
            }
        }
    }

    fn btc(btc: f64) -> bitcoin::Amount {
        bitcoin::Amount::from_btc(btc).unwrap()
    }

    fn btc_asset(amount: f64) -> bitcoin::Asset {
        bitcoin::Asset {
            amount: btc(amount),
            network: bitcoin::Network::Bitcoin,
        }
    }

    fn dai_amount(dai: f64) -> dai::Amount {
        dai::Amount::from_dai_trunc(dai).unwrap()
    }

    fn some_btc(btc: f64) -> Option<bitcoin::Amount> {
        Some(bitcoin::Amount::from_btc(btc).unwrap())
    }

    fn some_dai(dai: f64) -> Option<dai::Amount> {
        Some(dai::Amount::from_dai_trunc(dai).unwrap())
    }

    fn some_rate(rate: f64) -> Option<MidMarketRate> {
        Some(MidMarketRate::new(Rate::try_from(rate).unwrap()))
    }

    fn spread(spread: u16) -> Spread {
        Spread::new(spread).unwrap()
    }

    fn dai_asset(amount: dai::Amount) -> dai::Asset {
        dai::Asset {
            amount,
            contract_address: DaiContractAddress::Mainnet,
            chain_id: ChainId::mainnet(),
        }
    }

    #[test]
    fn btc_funds_reserved_upon_taking_sell_order() {
        let mut maker = Maker {
            btc_balance: some_btc(3.0),
            btc_fee: bitcoin::Amount::ZERO,
            ..Default::default()
        };

        let taken_order = TakenOrder {
            inner: BtcDaiOrder {
                position: Position::Sell,
                base: btc_asset(1.5),
                quote: dai_asset(dai::Amount::zero()),
            },
            ..Default::default()
        };

        let event = maker.process_taken_order(taken_order).unwrap();

        assert_eq!(event, TakeRequestDecision::GoForSwap);
        assert_eq!(maker.btc_reserved_funds, btc(1.5))
    }

    #[test]
    fn btc_funds_reserved_upon_taking_sell_order_with_fee() {
        let mut maker = Maker {
            btc_balance: some_btc(3.0),
            btc_fee: btc(1.0),
            ..Default::default()
        };

        let taken_order = TakenOrder {
            inner: BtcDaiOrder {
                position: Position::Sell,
                base: btc_asset(1.5),
                quote: dai_asset(dai::Amount::zero()),
            },
            ..Default::default()
        };

        let event = maker.process_taken_order(taken_order).unwrap();

        assert_eq!(event, TakeRequestDecision::GoForSwap);
        assert_eq!(maker.btc_reserved_funds, btc(2.5))
    }

    #[test]
    fn dai_funds_reserved_upon_taking_buy_order() {
        let mut maker = Maker {
            dai_balance: some_dai(10000.0),
            mid_market_rate: some_rate(1.5),
            ..Default::default()
        };

        let taken_order = TakenOrder {
            inner: BtcDaiOrder {
                position: Position::Buy,
                base: btc_asset(1.0),
                quote: dai_asset(dai_amount(1.5)),
            },
            ..Default::default()
        };

        let result = maker.process_taken_order(taken_order).unwrap();

        assert_eq!(result, TakeRequestDecision::GoForSwap);
        assert_eq!(maker.dai_reserved_funds, dai_amount(1.5))
    }

    #[test]
    fn dai_funds_reserved_upon_taking_buy_order_with_fee() {
        let mut maker = Maker {
            dai_balance: some_dai(10000.0),
            mid_market_rate: some_rate(1.5),
            ..Default::default()
        };

        let taken_order = TakenOrder {
            inner: BtcDaiOrder {
                position: Position::Buy,
                base: btc_asset(1.0),
                quote: dai_asset(dai_amount(1.5)),
            },
            ..Default::default()
        };

        let result = maker.process_taken_order(taken_order).unwrap();

        assert_eq!(result, TakeRequestDecision::GoForSwap);
        assert_eq!(maker.dai_reserved_funds, dai_amount(1.5))
    }

    #[test]
    fn not_enough_btc_funds_to_reserve_for_a_sell_order() {
        let mut maker = Maker {
            btc_balance: Some(bitcoin::Amount::ZERO),
            ..Default::default()
        };

        let taken_order = TakenOrder {
            inner: BtcDaiOrder {
                position: Position::Sell,
                base: btc_asset(1.5),
                quote: dai_asset(dai::Amount::zero()),
            },
            ..Default::default()
        };

        let result = maker.process_taken_order(taken_order).unwrap();

        assert_eq!(result, TakeRequestDecision::InsufficientFunds);
    }

    #[test]
    fn not_enough_btc_funds_to_reserve_for_a_buy_order() {
        let mut maker = Maker {
            dai_balance: Some(dai::Amount::zero()),
            mid_market_rate: some_rate(1.5),
            ..Default::default()
        };

        let taken_order = TakenOrder {
            inner: BtcDaiOrder {
                position: Position::Buy,
                base: btc_asset(1.0),
                quote: dai_asset(dai_amount(1.5)),
            },
            ..Default::default()
        };

        let result = maker.process_taken_order(taken_order).unwrap();

        assert_eq!(result, TakeRequestDecision::InsufficientFunds);
    }

    #[test]
    fn not_enough_btc_funds_to_reserve_for_a_sell_order_2() {
        let mut maker = Maker {
            btc_balance: some_btc(2.0),
            btc_reserved_funds: btc(1.5),
            ..Default::default()
        };

        let taken_order = TakenOrder {
            inner: BtcDaiOrder {
                position: Position::Sell,
                base: btc_asset(1.0),
                quote: dai_asset(dai::Amount::zero()),
            },
            ..Default::default()
        };

        let result = maker.process_taken_order(taken_order).unwrap();

        assert_eq!(result, TakeRequestDecision::InsufficientFunds);
    }

    #[test]
    fn yield_error_if_rate_is_not_available() {
        let mut maker = Maker {
            mid_market_rate: None,
            ..Default::default()
        };

        let taken_order = TakenOrder {
            ..Default::default()
        };

        let result = maker.process_taken_order(taken_order);
        assert!(result.is_err());

        let result = maker.new_buy_order();
        assert!(result.is_err());

        let result = maker.new_sell_order();
        assert!(result.is_err());
    }

    #[test]
    fn fail_to_confirm_sell_order_if_sell_rate_is_not_good_enough() {
        let mut maker = Maker {
            mid_market_rate: some_rate(10000.0),
            ..Default::default()
        };

        let taken_order = TakenOrder {
            inner: BtcDaiOrder {
                position: Position::Sell,
                base: btc_asset(1.0),
                quote: dai_asset(dai_amount(9000.0)),
            },
            ..Default::default()
        };

        let result = maker.process_taken_order(taken_order).unwrap();

        assert_eq!(result, TakeRequestDecision::RateNotProfitable);
    }

    #[test]
    fn fail_to_confirm_buy_order_if_buy_rate_is_not_good_enough() {
        let mut maker = Maker {
            mid_market_rate: some_rate(10000.0),
            ..Default::default()
        };

        let taken_order = TakenOrder {
            inner: BtcDaiOrder {
                position: Position::Buy,
                base: btc_asset(1.0),
                quote: dai_asset(dai_amount(11000.0)),
            },
            ..Default::default()
        };

        let result = maker.process_taken_order(taken_order).unwrap();

        assert_eq!(result, TakeRequestDecision::RateNotProfitable);
    }

    #[test]
    fn no_rate_change_if_rate_update_with_same_value() {
        let init_rate = some_rate(1.0);
        let mut maker = Maker {
            mid_market_rate: init_rate,
            ..Default::default()
        };

        let new_mid_market_rate = MidMarketRate::new(Rate::try_from(1.0).unwrap());

        let result = maker.update_rate(new_mid_market_rate).unwrap();
        assert!(result.is_none());
        assert_eq!(maker.mid_market_rate, init_rate)
    }

    #[test]
    fn rate_updated_and_new_orders_if_rate_update_with_new_value() {
        let mut maker = Maker {
            btc_balance: some_btc(10.0),
            dai_balance: some_dai(10.0),
            mid_market_rate: some_rate(1.0),
            ..Default::default()
        };

        let new_mid_market_rate = MidMarketRate::new(Rate::try_from(2.0).unwrap());

        let result = maker.update_rate(new_mid_market_rate).unwrap();
        assert!(result.is_some());
        assert_eq!(maker.mid_market_rate, Some(new_mid_market_rate))
    }

    #[test]
    fn free_funds_when_processing_finished_swap() {
        let mut maker = Maker {
            btc_reserved_funds: btc(1.1),
            dai_reserved_funds: dai_amount(1.0),
            btc_fee: btc(0.1),
            ..Default::default()
        };

        let free_btc = Some(btc(0.5));
        maker.free_funds(None, free_btc);
        assert_eq!(maker.btc_reserved_funds, btc(0.5));

        let free_dai = Some(dai_amount(0.5));
        maker.free_funds(free_dai, None);
        assert_eq!(maker.dai_reserved_funds, dai_amount(0.5));
    }

    #[test]
    fn no_new_sell_order_if_no_btc_balance_change() {
        let mut maker = Maker {
            btc_balance: some_btc(1.0),
            ..Default::default()
        };

        let result = maker.update_bitcoin_balance(btc(1.0)).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn no_new_buy_order_if_no_dai_balance_change() {
        let mut maker = Maker {
            dai_balance: some_dai(1.0),
            ..Default::default()
        };

        let result = maker.update_dai_balance(dai_amount(1.0)).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn new_sell_order_if_btc_balance_change() {
        let mut maker = Maker {
            btc_balance: some_btc(1.0),
            btc_max_sell_amount: None,
            btc_fee: bitcoin::Amount::ZERO,
            mid_market_rate: some_rate(1.0),
            spread: spread(0),
            ..Default::default()
        };
        let new_balance = btc(0.5);

        let new_sell_order = maker.update_bitcoin_balance(new_balance).unwrap().unwrap();
        assert_eq!(new_sell_order.position, Position::Sell);
        assert_eq!(maker.btc_balance, Some(new_balance))
    }

    #[test]
    fn new_buy_order_if_dai_balance_change() {
        let mut maker = Maker {
            dai_balance: some_dai(1.0),
            dai_max_sell_amount: None,
            mid_market_rate: some_rate(1.0),
            spread: spread(0),
            ..Default::default()
        };
        let new_balance = dai_amount(0.5);

        let new_buy_order = maker
            .update_dai_balance(new_balance.clone())
            .unwrap()
            .unwrap();
        assert_eq!(new_buy_order.position, Position::Buy);
        assert_eq!(maker.dai_balance, Some(new_balance))
    }

    #[test]
    fn published_sell_order_can_be_taken() {
        let mut maker = Maker {
            btc_balance: some_btc(3.0),
            btc_fee: bitcoin::Amount::ZERO,
            btc_max_sell_amount: some_btc(1.0),
            mid_market_rate: some_rate(1.0),
            ..Default::default()
        };

        let new_sell_order = maker.new_sell_order().unwrap();
        assert_eq!(new_sell_order.base, btc_asset(1.0));

        let taker = Taker::default();
        let result = maker
            .process_taken_order(TakenOrder {
                inner: new_sell_order,
                taker,
            })
            .unwrap();

        assert_eq!(result, TakeRequestDecision::GoForSwap);
        assert_eq!(maker.btc_reserved_funds, btc(1.0))
    }

    #[test]
    fn published_buy_order_can_be_taken() {
        let mut maker = Maker {
            dai_balance: some_dai(3.0),
            dai_max_sell_amount: some_dai(1.0),
            mid_market_rate: some_rate(1.0),
            ..Default::default()
        };

        let new_buy_order = maker.new_buy_order().unwrap();
        assert_eq!(new_buy_order.quote, dai_asset(dai_amount(1.0)));

        let taker = Taker::default();
        let result = maker
            .process_taken_order(TakenOrder {
                inner: new_buy_order,
                taker,
            })
            .unwrap();

        assert_eq!(result, TakeRequestDecision::GoForSwap);
        assert_eq!(maker.dai_reserved_funds, dai_amount(1.0))
    }

    #[test]
    fn new_buy_order_is_correct() {
        let maker = Maker {
            dai_balance: some_dai(20.0),
            dai_max_sell_amount: some_dai(18.0),
            mid_market_rate: some_rate(9000.0),
            btc_balance: some_btc(1.0),
            ..Default::default()
        };

        let new_buy_order = maker.new_buy_order().unwrap();
        assert_eq!(new_buy_order.base.amount, btc(0.002));
        assert_eq!(new_buy_order.quote.amount, dai_amount(18.0));
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::ethereum::ChainId;
    use crate::rate::Rate;
    use libp2p::PeerId;
    use std::convert::TryFrom;

    fn btc(btc: f64) -> bitcoin::Amount {
        bitcoin::Amount::from_btc(btc).unwrap()
    }

    fn btc_asset(amount: f64) -> bitcoin::Asset {
        bitcoin::Asset {
            amount: btc(amount),
            network: bitcoin::Network::Bitcoin,
        }
    }

    fn dai_amount(dai: f64) -> dai::Amount {
        dai::Amount::from_dai_trunc(dai).unwrap()
    }

    fn some_btc(btc: f64) -> Option<bitcoin::Amount> {
        Some(bitcoin::Amount::from_btc(btc).unwrap())
    }

    fn some_dai(dai: f64) -> Option<dai::Amount> {
        Some(dai::Amount::from_dai_trunc(dai).unwrap())
    }

    fn some_rate(rate: f64) -> Option<MidMarketRate> {
        Some(MidMarketRate::new(Rate::try_from(rate).unwrap()))
    }

    fn spread(spread: u16) -> Spread {
        Spread::new(spread).unwrap()
    }

    fn dai_asset(amount: dai::Amount) -> dai::Asset {
        dai::Asset {
            amount,
            contract_address: DaiContractAddress::Mainnet,
            chain_id: ChainId::mainnet(),
        }
    }

    #[test]
    fn take_order_update_balance() {
        // init maker

        let initial_mid_market_rate = some_rate(9000.0);
        let mut maker = Maker::new(
            btc(3.0),
            dai_amount(27_000.0),
            btc(0.0001),
            some_btc(1.0001),
            some_dai(8_550.0),
            initial_mid_market_rate.unwrap(),
            Spread::new(500).unwrap(),
            DaiContractAddress::Mainnet,
            bitcoin::Network::Bitcoin,
            ethereum::ChainId::mainnet(),
        );

        let initial_buy_order = maker.new_buy_order().unwrap();
        let initial_sell_order = maker.new_sell_order().unwrap();

        let expected_initial_buy_order = BtcDaiOrder {
            position: Position::Buy,
            base: btc_asset(1.0), // profitable_rate = 8550, base_amount = 8550 / 8550
            quote: dai_asset(dai_amount(8_550.0)),
        };
        let expected_initial_sell_order = BtcDaiOrder {
            position: Position::Sell,
            base: btc_asset(1.0),                  // deduct fee
            quote: dai_asset(dai_amount(9_450.0)), // profitable_rate = 9450, quote_amount = 0.9999 * 9450
        };

        assert_eq!(initial_buy_order, expected_initial_buy_order);
        assert_eq!(initial_sell_order, expected_initial_sell_order);

        // take request for the initial SELL order
        let taker_order = TakenOrder {
            inner: initial_sell_order.clone(),
            taker: Taker::new(PeerId::random()),
        };

        let result = maker.process_taken_order(taker_order).unwrap();

        assert_eq!(result, TakeRequestDecision::GoForSwap);

        let new_sell_order = maker.new_sell_order().unwrap();
        let new_buy_order = maker.new_buy_order().unwrap();

        // We still have enough more funds than max available on both sides, thus the orders should still be the same.
        assert_eq!(new_buy_order, initial_buy_order);
        assert_eq!(new_sell_order, initial_sell_order);

        // take request for the initial BUY order
        let taker_order = TakenOrder {
            inner: initial_buy_order.clone(),
            taker: Taker::new(PeerId::random()),
        };

        let result = maker.process_taken_order(taker_order).unwrap();

        assert_eq!(result, TakeRequestDecision::GoForSwap);

        let new_sell_order = maker.new_sell_order().unwrap();
        let new_buy_order = maker.new_buy_order().unwrap();

        // We still have enough more funds than max available on both sides, thus the orders should still be the same.
        assert_eq!(new_buy_order, initial_buy_order);
        assert_eq!(new_sell_order, initial_sell_order);

        // TODO take another sell and assert on new sell order (less than max amount)

        // TODO take another buy and assert on buy sell order (since we don't handle fees should still be max amount)

        // TODO simulate balance change(s) after a swap and assert on order(s)
    }
}
