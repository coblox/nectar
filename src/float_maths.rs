use bitcoin::hashes::core::cmp::Ordering;
use num::BigUint;
use std::str::FromStr;

/// Truncate the float's mantissa to length `precision`.
pub fn truncate(float: f64, precision: u16) -> f64 {
    let mut string = float.to_string();
    let index = string.find('.');

    match index {
        None => float,
        Some(index) => {
            let trunc = index + 1 + precision as usize;
            string.truncate(trunc);
            f64::from_str(&string).expect("This should still be a number")
        }
    }
}

/// Multiple float by 10e`pow`, Returns as a BigUint. No data loss.
/// Errors if the float is negative.
/// Errors if the result is a fraction.
pub fn multiple_pow_ten(float: f64, pow: u16) -> anyhow::Result<BigUint> {
    if float.is_sign_negative() {
        anyhow::bail!("Float is negative");
    }
    let mut float = float.to_string();
    let decimal_index = float.find('.');

    match decimal_index {
        None => {
            let zeroes = "0".repeat(pow as usize);
            Ok(BigUint::from_str(&format!("{}{}", float, zeroes)).expect("an integer"))
        }
        Some(decimal_index) => {
            let mantissa = float.split_off(decimal_index + 1);
            // Removes the decimal point
            float.truncate(float.len() - 1);
            let integer = float;

            if mantissa.is_empty() {
                unreachable!("already covered with decimal_index == None");
            } else {
                let pow = pow as usize;
                match mantissa.len().cmp(&pow) {
                    Ordering::Less => {
                        let remain = pow as usize - mantissa.len();
                        let zeroes = "0".repeat(remain);
                        Ok(
                            BigUint::from_str(&format!("{}{}{}", integer, mantissa, zeroes))
                                .expect("an integer"),
                        )
                    }
                    Ordering::Equal => {
                        Ok(BigUint::from_str(&format!("{}{}", integer, mantissa))
                            .expect("an integer"))
                    }
                    Ordering::Greater => anyhow::bail!("Result is not an integer"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn it_truncates() {
        let float = 1.123456789;

        assert_eq!(&truncate(float, 5).to_string(), "1.12345");
    }

    proptest! {
        #[test]
        fn truncate_doesnt_panic(f in any::<f64>(), p in any::<u16>()) {
               truncate(f, p);
        }
    }

    #[test]
    fn given_integer_then_it_multiplies() {
        let float = 123_456_789.0f64;
        let pow = 6;

        assert_eq!(
            multiple_pow_ten(float, pow).unwrap(),
            BigUint::from(123_456_789_000_000u64)
        )
    }

    #[test]
    fn given_mantissa_of_pow_length_then_it_multiplies() {
        let float = 123.123_456_789f64;
        let pow = 9;

        assert_eq!(
            multiple_pow_ten(float, pow).unwrap(),
            BigUint::from(123_123_456_789u64)
        )
    }

    #[test]
    fn given_mantissa_length_lesser_than_pow_then_it_multiplies() {
        let float = 123.123_456_789f64;
        let pow = 12;

        assert_eq!(
            multiple_pow_ten(float, pow).unwrap(),
            BigUint::from(123_123_456_789_000u64)
        )
    }

    #[test]
    fn given_mantissa_length_greater_than_pow_then_it_errors() {
        let float = 123.123_456_789f64;
        let pow = 6;

        assert!(multiple_pow_ten(float, pow).is_err(),)
    }

    #[test]
    fn given_negative_float_then_it_errors() {
        let float = -123_456_789.0f64;
        let pow = 6;

        assert!(multiple_pow_ten(float, pow).is_err(),)
    }

    proptest! {
        #[test]
        fn multiple_pow_ten_doesnt_panic(f in any::<f64>(), p in any::<u16>()) {
               let _ = multiple_pow_ten(f, p);
        }
    }
}
