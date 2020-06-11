use crate::dai;
use crate::dai::ATTOS_IN_DAI_EXP;
use crate::float_maths::multiple_pow_ten;
use crate::publish::WorthIn;
use num::pow::Pow;
use num::BigUint;

pub const SATS_IN_BITCOIN_EXP: u16 = 8;

#[derive(Debug, Copy, Clone, Ord, PartialOrd, PartialEq, Eq)]
pub struct Amount(::bitcoin::Amount);

impl Amount {
    pub fn from_btc(btc: f64) -> anyhow::Result<Amount> {
        Ok(Amount(::bitcoin::Amount::from_btc(btc)?))
    }

    pub fn from_sat(sat: u64) -> Self {
        Amount(::bitcoin::Amount::from_sat(sat))
    }

    pub fn as_sat(self) -> u64 {
        self.0.as_sat()
    }

    pub fn as_btc(self) -> f64 {
        self.0.as_btc()
    }
}

impl WorthIn<dai::Amount> for Amount {
    const MAX_PRECISION_EXP: u16 = 9;

    fn worth_in(&self, btc_to_dai_rate: f64) -> anyhow::Result<dai::Amount> {
        if btc_to_dai_rate.is_sign_negative() {
            anyhow::bail!("Rate is negative.");
        }

        if btc_to_dai_rate <= 10e-10 {
            anyhow::bail!("Rate is null.");
        }

        if btc_to_dai_rate.is_infinite() {
            anyhow::bail!("Rate is infinite.");
        }

        let uint_rate =
            multiple_pow_ten(btc_to_dai_rate, Self::MAX_PRECISION_EXP).map_err(|_| {
                anyhow::anyhow!("Rate's precision is too high, truncation would ensue.")
            })?;

        // Apply the rate
        let worth = uint_rate * self.as_sat();

        // The rate input is for bitcoin to dai but we applied to satoshis so we need to:
        // - divide to get bitcoins
        // - divide to adjust for max_precision
        // - multiple to get attodai
        let adjustment_exp =
            BigUint::from(ATTOS_IN_DAI_EXP - Self::MAX_PRECISION_EXP - SATS_IN_BITCOIN_EXP);

        let adjustment = BigUint::from(10u64).pow(adjustment_exp);

        let atto_dai = worth * adjustment;

        Ok(dai::Amount::from_atto(atto_dai))
    }
}

impl std::ops::Sub for Amount {
    type Output = Amount;

    fn sub(self, rhs: Self) -> Self::Output {
        Amount(self.0 - rhs.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn using_too_precise_rate_returns_error() {
        let btc = Amount::from_btc(1.0).unwrap();

        let res: anyhow::Result<dai::Amount> = btc.worth_in(1000.1234567891);

        assert!(res.is_err())
    }

    #[test]
    fn using_rate_returns_correct_result() {
        let btc = Amount::from_btc(1.0).unwrap();

        let res: dai::Amount = btc.worth_in(1000.123456789).unwrap();

        assert_eq!(res, dai::Amount::from_dai_trunc(1000.123456789).unwrap());
    }
}
