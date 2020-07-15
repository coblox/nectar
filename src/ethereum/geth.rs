use crate::jsonrpc;
use anyhow::Context;
use asset::Erc20Quantity;
use comit::{
    asset::{self, ethereum::TryFromWei},
    ethereum::{Address, ChainId, Hash, Transaction, TransactionReceipt},
};
use ethereum_types::U256;
use num::{BigUint, Num};
use serde_hex::{CompactPfx, SerHex, SerHexSeq, StrictPfx};

pub const JSONRPC_VERSION: &str = "2.0";

#[derive(Debug, Clone)]
pub struct Client {
    rpc_client: jsonrpc::Client,
}

impl Client {
    pub fn new(url: url::Url) -> Self {
        Client {
            rpc_client: jsonrpc::Client::new(url),
        }
    }

    pub async fn chain_id(&self) -> anyhow::Result<ChainId> {
        let chain_id = self
            .rpc_client
            .send::<Vec<()>, String>(jsonrpc::Request::new(
                "net_version",
                vec![],
                JSONRPC_VERSION.into(),
            ))
            .await
            .context("failed to fetch net version")?;
        let chain_id: u32 = chain_id.parse()?;
        let chain_id = ChainId::from(chain_id);

        Ok(chain_id)
    }

    pub async fn send_transaction(&self, request: SendTransactionRequest) -> anyhow::Result<Hash> {
        let tx_hash = self
            .rpc_client
            .send(jsonrpc::Request::new(
                "eth_sendTransaction",
                vec![jsonrpc::serialize(request)?],
                JSONRPC_VERSION.into(),
            ))
            .await
            .context("failed to send transaction")?;

        Ok(tx_hash)
    }

    pub async fn send_raw_transaction(&self, transaction_hex: String) -> anyhow::Result<Hash> {
        let tx_hash = self
            .rpc_client
            .send(jsonrpc::Request::new(
                "eth_sendRawTransaction",
                vec![transaction_hex],
                JSONRPC_VERSION.into(),
            ))
            .await
            .context("failed to send raw transaction")?;

        Ok(tx_hash)
    }

    pub async fn get_transaction_by_hash(
        &self,
        transaction_hash: Hash,
    ) -> anyhow::Result<Transaction> {
        let transaction = self
            .rpc_client
            .send(jsonrpc::Request::new(
                "eth_getTransactionByHash",
                vec![jsonrpc::serialize(transaction_hash)?],
                JSONRPC_VERSION.into(),
            ))
            .await
            .context("failed to get transaction by hash")?;

        Ok(transaction)
    }

    pub async fn get_transaction_receipt(
        &self,
        transaction_hash: Hash,
    ) -> anyhow::Result<TransactionReceipt> {
        let receipt = self
            .rpc_client
            .send(jsonrpc::Request::new(
                "eth_getTransactionReceipt",
                vec![jsonrpc::serialize(transaction_hash)?],
                JSONRPC_VERSION.into(),
            ))
            .await
            .context("failed to get transaction receipt")?;

        Ok(receipt)
    }

    pub async fn get_transaction_count(&self, account: Address) -> anyhow::Result<u32> {
        let count: String = self
            .rpc_client
            .send(jsonrpc::Request::new(
                "eth_getTransactionCount",
                vec![jsonrpc::serialize(account)?, jsonrpc::serialize("latest")?],
                JSONRPC_VERSION.into(),
            ))
            .await
            .context("failed to get transaction count")?;

        let count = u32::from_str_radix(&count[2..], 16)?;
        Ok(count)
    }

    pub async fn erc20_balance(
        &self,
        account: Address,
        token_contract: Address,
    ) -> anyhow::Result<asset::Erc20> {
        #[derive(Debug, serde::Serialize)]
        struct CallRequest {
            to: Address,
            #[serde(with = "SerHexSeq::<StrictPfx>")]
            data: Vec<u8>,
        }

        let call_request = CallRequest {
            to: token_contract,
            data: balance_of_fn(account)?,
        };

        let quantity: String = self
            .rpc_client
            .send(jsonrpc::Request::new(
                "eth_call",
                vec![
                    jsonrpc::serialize(call_request)?,
                    jsonrpc::serialize("latest")?,
                ],
                JSONRPC_VERSION.into(),
            ))
            .await
            .context("failed to get erc20 token balance")?;
        let quantity = BigUint::from_str_radix(&quantity[2..], 16)?;
        let quantity = Erc20Quantity::try_from_wei(quantity)?;

        Ok(asset::Erc20 {
            token_contract,
            quantity,
        })
    }
}

fn balance_of_fn(account: Address) -> anyhow::Result<Vec<u8>> {
    let account = clarity::Address::from_slice(account.as_bytes())
        .map_err(|_| anyhow::anyhow!("Could not construct clarity::Address from slice"))?;

    let balance_of = clarity::abi::encode_call(
        "balanceOf(address)",
        &[clarity::abi::Token::Address(account)],
    );

    Ok(balance_of)
}

#[derive(Debug, serde::Serialize)]
pub struct SendTransactionRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Address>,
    pub value: U256,
    #[serde(with = "SerHex::<CompactPfx>")]
    pub gas_limit: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<u8>>,
}

#[cfg(all(test, feature = "test-docker"))]
mod test {
    use super::*;
    use crate::test_harness::ethereum;
    use testcontainers::clients;

    #[tokio::test]
    async fn get_chain_id() {
        let tc_client = clients::Cli::default();
        let blockchain = ethereum::Blockchain::new(&tc_client).unwrap();

        let client = Client::new(blockchain.node_url);

        let chain_id = client.chain_id().await.unwrap();

        assert_eq!(chain_id, ChainId::regtest())
    }
}