use crate::bitcoin::{self, SATS_IN_BITCOIN_EXP};
use crate::float_maths::{divide_pow_ten_trunc, multiple_pow_ten, truncate};
use crate::publish::WorthIn;
use num::{pow::Pow, BigUint, ToPrimitive};
use std::ops::{Div, Mul};

pub const ATTOS_IN_DAI_EXP: u16 = 18;

lazy_static::lazy_static! {
    pub static ref DAI_DEC: BigUint =
        BigUint::from(10u16).pow(ATTOS_IN_DAI_EXP);
}

// It means the mantissa can be up to 9 digits long
const DAI_PRECISION_EXP: u16 = 9;

lazy_static::lazy_static! {
    pub static ref DAI_PRECISION: u32 =
        10u32.pow(DAI_PRECISION_EXP as u32);
}

#[derive(Clone, Ord, PartialOrd, PartialEq, Eq)]
pub struct Amount(BigUint);

impl Amount {
    /// Rounds the value received to a 9 digits mantissa.
    pub fn from_dai_trunc(dai: f64) -> anyhow::Result<Self> {
        if dai.is_sign_negative() {
            anyhow::bail!("Passed value is negative")
        }

        let dai = truncate(dai, DAI_PRECISION_EXP);

        let u_int_value = multiple_pow_ten(dai, DAI_PRECISION_EXP).expect("It is truncated");

        Ok(Amount(DAI_DEC.clone().mul(u_int_value).div(*DAI_PRECISION)))
    }

    pub fn from_atto(atto: BigUint) -> Self {
        Amount(atto)
    }

    pub fn as_atto(&self) -> BigUint {
        self.0.clone()
    }
}

impl std::fmt::Debug for Amount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Display for Amount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl WorthIn<crate::bitcoin::Amount> for Amount {
    const MAX_PRECISION_EXP: u16 = 6;

    fn worth_in(&self, dai_to_btc_rate: f64) -> anyhow::Result<bitcoin::Amount> {
        if dai_to_btc_rate.is_sign_negative() {
            anyhow::bail!("Rate is negative.");
        }

        if dai_to_btc_rate <= 10e-10 {
            anyhow::bail!("Rate is null.");
        }

        if dai_to_btc_rate.is_infinite() {
            anyhow::bail!("Rate is infinite.");
        }

        let uint_rate =
            multiple_pow_ten(dai_to_btc_rate, Self::MAX_PRECISION_EXP).map_err(|_| {
                anyhow::anyhow!("Rate's precision is too high, truncation would ensue.")
            })?;

        // Apply the rate
        let worth = uint_rate * self.as_atto();

        // The rate input is for dai to bitcoin but we applied it to attodai so we need to:
        // - divide to get dai
        // - divide to adjust for max_precision
        // - multiple to get satoshis
        // Note that we are doing the inverse of that to then pass it to divide_pow_ten_trunc
        let inv_adjustment_exp = Self::MAX_PRECISION_EXP + ATTOS_IN_DAI_EXP - SATS_IN_BITCOIN_EXP;

        // We may truncate here if self contains an attodai amount which is too precise
        let sats = divide_pow_ten_trunc(worth, inv_adjustment_exp);

        let sats = sats
            .to_u64()
            .ok_or_else(|| anyhow::anyhow!("Result is unexpectedly large"))?;

        Ok(bitcoin::Amount::from_sat(sats))
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
    fn given_float_dai_amount_with_mantissa_of_length_nine_then_exact_value_is_stored() {
        let some_dai = Amount::from_dai_trunc(1.555_555_555).unwrap();
        let same_amount = Amount::from_atto(BigUint::from(1_555_555_555_000_000_000u64));

        assert_eq!(some_dai, same_amount);
    }

    #[test]
    fn given_float_dai_amount_with_mantissa_of_length_ten_then_truncated_value_is_stored() {
        let some_dai = Amount::from_dai_trunc(1.555_555_555_5).unwrap();
        let same_amount = Amount::from_atto(BigUint::from(1_555_555_555_000_000_000u64));

        assert_eq!(some_dai, same_amount);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn using_too_precise_rate_returns_error() {
        let dai = Amount::from_dai_trunc(1.0).unwrap();

        let res: anyhow::Result<bitcoin::Amount> = dai.worth_in(0.1234567);

        assert!(res.is_err())
    }

    #[test]
    fn using_rate_returns_correct_result() {
        let dai = Amount::from_dai_trunc(1.0).unwrap();

        let res: bitcoin::Amount = dai.worth_in(0.001234).unwrap();

        assert_eq!(res, bitcoin::Amount::from_btc(0.001234).unwrap());
    }

    proptest! {
        #[test]
        fn doesnt_panic(f in any::<f64>()) {
               let _ = Amount::from_dai_trunc(f);
        }
    }
}
