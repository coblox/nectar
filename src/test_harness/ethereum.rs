use crate::ethereum::{self, ether, Address, ChainId};
use anyhow::Context;
use clarity::PrivateKey;
use comit::{
    actions::ethereum::DeployContract,
    asset::{Erc20, Erc20Quantity, Ether},
};
use num256::Uint256;
use std::str::FromStr;
use testcontainers::{
    clients,
    images::generic::{GenericImage, Stream, WaitFor},
    Container, Docker, Image,
};
use url::Url;

pub const TOKEN_CONTRACT: &str = include_str!("./erc20_token/contract.hex");
pub const GETH_HOST_KEYSTORE_DIR: &str = "./.geth_datadir";

// We can decrypt the private key from the file in
// "../../.geth_datadir/", but it takes more than 1 minute, which
// slows down the tests unnecessarily
pub const GETH_DEV_ACCOUNT_PRIVATE_KEY: &str =
    "0x0bad9cdf7205a60039d5034b38cdadbbfc5e4f1c7436da011dd7d8c7684bcb1c";

#[derive(Debug)]
pub struct Blockchain<'c> {
    _container: Container<'c, clients::Cli, GenericImage>,
    dev_account_wallet: ethereum::Wallet,
    pub node_url: Url,
}

impl<'c> Blockchain<'c> {
    pub fn new(client: &'c clients::Cli) -> anyhow::Result<Self> {
        let geth_image = GenericImage::new("ethereum/client-go:v1.9.13")
            .with_wait_for(WaitFor::LogMessage {
                message: String::from("mined potential block"),
                stream: Stream::StdErr,
            })
            .with_args(vec![
                String::from("--dev"),
                String::from("--dev.period=1"),
                String::from("--networkid=1337"),
                String::from("--rpc"),
                String::from("--rpcaddr=0.0.0.0"),
                String::from("--rpcport=8545"),
                String::from("--verbosity=5"),
                String::from("--keystore=.ethereum"),
                String::from("--rpcapi=db,eth,net,web3,personal"),
            ])
            .with_volume(
                std::fs::canonicalize(GETH_HOST_KEYSTORE_DIR)?
                    .to_str()
                    .expect("valid unicode path"),
                "/.ethereum/",
            );
        let container = client.run(geth_image);
        let port = container.get_host_port(8545);

        let url = format!("http://localhost:{}", port.unwrap());
        let url = Url::parse(&url)?;

        let dev_account_wallet = ethereum::Wallet::new_from_private_key(
            PrivateKey::from_str(GETH_DEV_ACCOUNT_PRIVATE_KEY).map_err(|_| {
                anyhow::anyhow!("Failed to parse geth dev account private key from string")
            })?,
            url.clone(),
            ChainId::GETH_DEV,
        );

        Ok(Self {
            _container: container,
            node_url: url,
            dev_account_wallet,
        })
    }

    pub async fn init(&mut self) -> anyhow::Result<()> {
        self.deploy_token_contract().await?;
        Ok(())
    }

    pub fn token_contract(&self) -> Address {
        self.dev_account_wallet.dai_contract_address()
    }

    pub fn chain_id(&self) -> ChainId {
        self.dev_account_wallet.chain_id()
    }

    pub async fn mint_ether(
        &self,
        to: Address,
        ether: ether::Amount,
        chain_id: ChainId,
    ) -> anyhow::Result<()> {
        let _ = self
            .dev_account_wallet
            .send_transaction(to, ether, Some(100_000), None, chain_id)
            .await?;

        Ok(())
    }

    pub async fn mint_erc20_token(
        &self,
        to: Address,
        asset: Erc20,
        chain_id: ChainId,
    ) -> anyhow::Result<()> {
        let transfer = self.transfer_fn(to, asset.quantity)?;

        let _ = self
            .dev_account_wallet
            .send_transaction(
                asset.token_contract,
                ether::Amount::zero(),
                Some(100_000),
                Some(transfer),
                chain_id,
            )
            .await?;

        Ok(())
    }

    fn transfer_fn(&self, to: Address, value: Erc20Quantity) -> anyhow::Result<Vec<u8>> {
        let to = clarity::Address::from_slice(to.as_bytes())
            .map_err(|_| anyhow::anyhow!("Could not construct clarity::Address from slice"))?;

        let transfer = clarity::abi::encode_call("transfer(address,uint256)", &[
            clarity::abi::Token::Address(to),
            clarity::abi::Token::Uint(Uint256::from_bytes_le(value.to_bytes().as_slice())),
        ]);

        Ok(transfer)
    }

    async fn deploy_token_contract(&mut self) -> anyhow::Result<()> {
        let contract = TOKEN_CONTRACT[2..].trim(); // remove the 0x in the front and any whitespace
        let contract = hex::decode(contract).context("token contract should be valid hex")?;

        self.dev_account_wallet
            .deploy_dai_token_contract(DeployContract {
                data: contract,
                amount: Ether::zero(),
                gas_limit: 1_000_000,
                chain_id: ChainId::GETH_DEV,
            })
            .await?;

        Ok(())
    }
}
