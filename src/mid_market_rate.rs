use crate::Rate;
use chrono::{DateTime, Utc};
use std::convert::TryInto;

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct MidMarketRate {
    pub value: Rate,
    pub timestamp: DateTime<Utc>,
}

/// Get mid-market rate for the trading pair BTC-DAI.
///
/// Currently, this function only delegates to Kraken. Eventually, it
/// could return a value based on multiple sources.
pub async fn get_btc_dai_mid_market_rate() -> anyhow::Result<MidMarketRate> {
    kraken::get_btc_dai_mid_market_rate().await
}

#[cfg(test)]
impl Default for MidMarketRate {
    fn default() -> Self {
        Self {
            value: Rate::default(),
            timestamp: Utc::now(),
        }
    }
}

mod kraken {
    use super::*;
    use serde::de::Error;
    use serde::Deserialize;
    use std::convert::TryFrom;

    /// Fetch mid-market rate for the trading pair BTC-DAI from Kraken.
    ///
    /// More info here: https://www.kraken.com/features/api
    pub async fn get_btc_dai_mid_market_rate() -> anyhow::Result<MidMarketRate> {
        let ask_and_bid = reqwest::get("https://api.kraken.com/0/public/Ticker?pair=XBTDAI")
            .await?
            .json::<TickerResponse>()
            .await
            .map(|response| response.result.xbtdai)?;
        let rate = ask_and_bid.try_into()?;

        Ok(rate)
    }

    #[derive(Clone, Copy, Debug, Deserialize)]
    #[serde(try_from = "TickerData")]
    pub struct AskAndBid {
        pub ask: f64,
        pub bid: f64,
    }

    impl TryFrom<AskAndBid> for MidMarketRate {
        type Error = anyhow::Error;

        fn try_from(AskAndBid { ask, bid }: AskAndBid) -> anyhow::Result<Self> {
            let value = (bid + ask) / 2f64;
            let value = Rate::try_from(value).unwrap();

            Ok(Self {
                value,
                timestamp: Utc::now(),
            })
        }
    }

    #[derive(Deserialize)]
    struct TickerResponse {
        result: Ticker,
    }

    #[derive(Deserialize)]
    struct Ticker {
        #[serde(rename = "XBTDAI")]
        xbtdai: AskAndBid,
    }

    #[derive(Deserialize)]
    struct TickerData {
        #[serde(rename = "a")]
        ask: Vec<String>,
        #[serde(rename = "b")]
        bid: Vec<String>,
    }

    impl TryFrom<TickerData> for AskAndBid {
        type Error = serde_json::Error;

        fn try_from(value: TickerData) -> Result<Self, Self::Error> {
            let ask_price = value
                .ask
                .first()
                .ok_or_else(|| serde_json::Error::custom("no ask price"))?;
            let bid_price = value
                .bid
                .first()
                .ok_or_else(|| serde_json::Error::custom("no bid price"))?;

            Ok(AskAndBid {
                ask: ask_price
                    .parse::<f64>()
                    .map_err(serde_json::Error::custom)?,
                bid: bid_price
                    .parse::<f64>()
                    .map_err(serde_json::Error::custom)?,
            })
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;

        const TICKER_EXAMPLE: &str = r#"{
    "error": [],
    "result": {
        "XBTDAI": {
            "a": [
                "9489.50000",
                "1",
                "1.000"
            ],
            "b": [
                "9462.70000",
                "1",
                "1.000"
            ],
            "c": [
                "9496.50000",
                "0.00220253"
            ],
            "v": [
                "0.19793959",
                "0.55769847"
            ],
            "p": [
                "9583.44469",
                "9593.15707"
            ],
            "t": [
                12,
                22
            ],
            "l": [
                "9496.50000",
                "9496.50000"
            ],
            "h": [
                "9594.90000",
                "9616.10000"
            ],
            "o": "9562.30000"
        }
    }
}"#;

        #[test]
        fn given_ticker_example_data_deserializes_correctly() {
            serde_json::from_str::<TickerResponse>(TICKER_EXAMPLE).unwrap();
        }
    }
}
