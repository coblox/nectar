#![warn(
    unused_extern_crates,
    missing_debug_implementations,
    missing_copy_implementations,
    rust_2018_idioms,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::fallible_impl_from,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::dbg_macro
)]
#![allow(dead_code)] // To be removed further down the line
#![forbid(unsafe_code)]

mod bitcoin;
mod bitcoin_wallet;
mod bitcoind;
mod dai;
mod jsonrpc;
mod ongoing_swaps;
mod publish;
mod swap;
mod truncate_float;

#[cfg(all(test, feature = "test-docker"))]
pub mod test_harness;

lazy_static::lazy_static! {
    pub static ref SECP: ::bitcoin::secp256k1::Secp256k1<::bitcoin::secp256k1::All> =
        ::bitcoin::secp256k1::Secp256k1::new();
}

fn main() {
    println!("Hello, world!");
}
