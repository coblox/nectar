use crate::bitcoin;
use crate::dai;
use std::cmp::min;

pub trait BitcoinLockedFunds {
    fn bitcoin_locked_funds(&self) -> bitcoin::Amount;
}

pub trait BitcoinBalance {
    fn bitcoin_balance(&self) -> bitcoin::Amount;
}

pub trait BitcoinFees {
    fn bitcoin_fees(&self) -> bitcoin::Amount;
}

struct DaiBitcoinOrder {
    pub buy_amount: dai::Amount,
    pub sell_amount: bitcoin::Amount,
}

/// Allow to know the worth of self in a different asset using
/// The given conversion rate.
/// MAX_PRECISION_EXP is the maximum precision allowed (number of digits after
/// the comma) for the rate passed in. This is to ensure that no precision is loss
/// or truncation done when doing the conversion.
pub trait WorthIn<Asset> {
    const MAX_PRECISION_EXP: u16;

    fn worth_in(&self, conversion_rate: f64) -> anyhow::Result<Asset>;
}

/// Contains a positive percentage value expressed in ratio: 1 is 100%
/// To avoid human errors, the max value is 1.
struct Spread(f64);

impl Spread {
    pub fn new(spread: f64) -> Result<Spread, ()> {
        if spread.is_sign_positive() && spread <= 1.0 {
            Ok(Spread(spread))
        } else {
            Err(())
        }
    }

    pub fn apply(&self, base_rate: f64) -> f64 {
        base_rate * (1.0 + self.0)
    }
}

/// The maker creates an order that defines how much he wants to buy for the amount he is selling.
/// order's buy amount = what the maker wants from a taker
/// order's sell amount = what the maker is offering to a taker
///
/// mid_market_rate is set as 1 sell => x buy, where x is the mid_market_rate
///
/// BTC-DAI: When selling 1 BTC we should buy 9000 DAI, mid_market_rate is 1:9000
/// Given BTC:DAI and the rate of 1:9000
///     selling 1.0 BTC with spread_pc of 3% => buy 9270 DAI
///     selling 0.5 BTC with spread_pc of 3% => buy 4635 DAI
/// Given DAI:BTC and a rate of 1:0.0001
///     selling 10000 DAI with spread_pc of 3% => buy 1.03 BTC
///     selling 1000 DAI with spread_pc of 3% => buy 0.103 DAI
///
fn new_dai_bitcoin_order<W, B>(
    bitcoin_wallet: W,
    book: B,
    max_sell_amount: bitcoin::Amount,
    mid_market_rate: f64,
    spread: Spread,
) -> DaiBitcoinOrder
where
    W: BitcoinBalance + BitcoinFees,
    B: BitcoinLockedFunds,
{
    let sell_amount = min(
        bitcoin_wallet.bitcoin_balance() - book.bitcoin_locked_funds(),
        max_sell_amount,
    ) - bitcoin_wallet.bitcoin_fees();

    let rate = spread.apply(mid_market_rate);
    let rate = crate::float_maths::truncate(
        rate,
        <bitcoin::Amount as WorthIn<dai::Amount>>::MAX_PRECISION_EXP,
    );

    let buy_amount = sell_amount.worth_in(rate).unwrap();

    DaiBitcoinOrder {
        sell_amount,
        buy_amount,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Copy, Clone)]
    struct Book {
        locked_funds: bitcoin::Amount,
    }

    #[derive(Copy, Clone)]
    struct Wallet {
        balance: bitcoin::Amount,
        fees: bitcoin::Amount,
    }

    impl Wallet {
        fn new<A: Into<bitcoin::Amount>>(balance: A, fees: A) -> Wallet {
            Wallet {
                balance: balance.into(),
                fees: fees.into(),
            }
        }
    }

    impl BitcoinBalance for Wallet {
        fn bitcoin_balance(&self) -> bitcoin::Amount {
            self.balance
        }
    }

    impl BitcoinFees for Wallet {
        fn bitcoin_fees(&self) -> bitcoin::Amount {
            self.fees
        }
    }

    impl Book {
        fn new<A: Into<bitcoin::Amount>>(locked_funds: A) -> Book {
            Book {
                locked_funds: locked_funds.into(),
            }
        }
    }

    impl BitcoinLockedFunds for Book {
        fn bitcoin_locked_funds(&self) -> bitcoin::Amount {
            self.locked_funds
        }
    }

    fn btc(btc: f64) -> bitcoin::Amount {
        bitcoin::Amount::from_btc(btc).unwrap()
    }

    fn dai(dai: f64) -> dai::Amount {
        dai::Amount::from_dai_trunc(dai).unwrap()
    }

    #[test]
    fn given_a_balance_return_order_selling_full_balance() {
        let wallet = Wallet::new(btc(10.0), btc(0.0));

        let book = Book::new(btc(0.0));

        let order = new_dai_bitcoin_order(wallet, book, btc(100.0), 1.0, Spread::new(0.0).unwrap());

        assert_eq!(order.sell_amount, btc(10.0));
    }

    #[test]
    fn given_a_balance_and_locked_funds_return_order_selling_available_balance() {
        let wallet = Wallet::new(btc(10.0), btc(0.0));

        let book = Book::new(btc(2.0));

        let order = new_dai_bitcoin_order(wallet, book, btc(100.0), 1.0, Spread::new(0.0).unwrap());

        assert_eq!(order.sell_amount, btc(8.0));
    }

    #[test]
    fn given_an_available_balance_and_a_max_amount_sell_min_of_either() {
        let wallet = Wallet::new(btc(10.0), btc(0.0));

        let book = Book::new(btc(2.0));

        let order = new_dai_bitcoin_order(wallet, book, btc(2.0), 1.0, Spread::new(0.0).unwrap());

        assert_eq!(order.sell_amount, btc(2.0));
    }

    #[test]
    fn given_an_available_balance_and_fees_sell_balance_minus_fees() {
        let wallet = Wallet::new(btc(10.0), btc(1.0));

        let book = Book::new(btc(2.0));

        let order = new_dai_bitcoin_order(wallet, book, btc(2.0), 1.0, Spread::new(0.0).unwrap());

        assert_eq!(order.sell_amount, btc(1.0));
    }

    #[test]
    fn given_a_rate_return_order_with_both_amounts() {
        let wallet = Wallet::new(btc(1051.0), btc(1.0));

        let book = Book::new(btc(50.0));

        let order =
            new_dai_bitcoin_order(wallet, book, btc(9999.0), 0.1, Spread::new(0.0).unwrap());

        // 1 Sell => 0.1 Buy
        // 1000 Sell => 100 Buy
        assert_eq!(order.sell_amount, btc(1000.0));
        assert_eq!(order.buy_amount, dai(100.0));

        let order =
            new_dai_bitcoin_order(wallet, book, btc(9999.0), 10.0, Spread::new(0.0).unwrap());

        assert_eq!(order.sell_amount, btc(1000.0));
        assert_eq!(order.buy_amount, dai(10_000.0));
    }

    #[test]
    fn given_a_rate_and_spread_return_order_with_both_amounts() {
        let wallet = Wallet::new(btc(1051.0), btc(1.0));

        let book = Book::new(btc(50.0));

        let order =
            new_dai_bitcoin_order(wallet, book, btc(9999.0), 0.1, Spread::new(0.03).unwrap());

        assert_eq!(order.sell_amount, btc(1000.0));
        assert_eq!(order.buy_amount, dai(103.0));
    }
}
