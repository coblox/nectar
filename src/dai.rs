use crate::float_maths::{multiple_pow_ten, truncate};
use crate::publish::WorthIn;
use num::{pow::Pow, BigUint};
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
    const MAX_PRECISION_EXP: u16 = 9;

    fn worth_in(&self, _rhs: f64) -> anyhow::Result<crate::bitcoin::Amount> {
        unimplemented!()
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

    proptest! {
        #[test]
        fn doesnt_panic(f in any::<f64>()) {
               let _ = Amount::from_dai_trunc(f);
        }
    }
}
