use crate::helpers::servers::get_server_url;
use crate::structs::token_data::*;
use crate::structs::user_data::*;
use base64::engine::general_purpose;
use base64::Engine;
use chrono::Utc;
use log::debug;
use log::warn;
use reqwest::Url;
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::error::Error as StdError;

use anyhow::{anyhow, Result};

pub struct AuthClient {
    client: reqwest::Client,
    oauth_server: Url,
    token_data: TokenData,
    pub symbols: HashMap<String, Value>,
    pub exchanges: Vec<String>,
    pub user_data: UserData,
    pub portfolio_number: String,
}

impl AuthClient {
    pub fn new(refresh_token: &str, demo: bool) -> Result<Self> {
        Ok(AuthClient {
            client: reqwest::Client::new(),
            oauth_server: Url::parse(get_server_url("oauth_server", demo)?.as_str())?,
            token_data: TokenData {
                raw_response: None,
                token: None,
                refresh_token: refresh_token.to_string(),
                token_decoded: None,
                token_issued: 0,
                token_ttl: 60,
            },
            symbols: HashMap::new(),
            exchanges: vec!["MOEX".to_string(), "SPBX".to_string()],
            user_data: UserData {
                accounts: Vec::new(),
            },
            portfolio_number: "".to_string(),
        })
    }

    pub async fn get_jwt_token(&mut self) -> Result<String, Box<dyn StdError>> {
        let now = Utc::now().timestamp();

        if (self.token_data.token.is_none())
            || (now - self.token_data.token_issued > self.token_data.token_ttl as i64)
        {
            let _ = self.get_jwt_token_force().await?;
            // let request_url = self.oauth_server.join("/refresh")?;

            // debug!("Sending RefreshToken: {}", self.token_data.refresh_token);
            // let response = self
            //     .client
            //     .post(request_url)
            //     .query(&vec![("token", self.token_data.refresh_token.clone())])
            //     .send()
            //     .await?;

            // if response.status().is_success() {
            //     // todo: remove raw_response and add validate response;
            //     let response_text = response.text().await?;
            //     let response_json: Value = serde_json::from_str(&*response_text)?;
            //     let token = response_json["AccessToken"].as_str().unwrap().to_string();
            //     self.token_data.raw_response = Some(response_text);
            //     self.token_data.token = Some(token.clone());
            //     self.token_data.token_decoded = Some(self.decode_token(token)?);
            //     self.token_data.token_issued = now;

            //     // todo: move to init?
            //     self.parse_data_from_token();
            // } else {
            //     self.token_data.token = None;
            //     self.token_data.token_decoded = None;
            //     self.token_data.token_issued = 0;
            // }
        }

        self.token_data.raw_response.clone().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                "JWT token refresh returned no raw response",
            )
            .into()
        })
    }

    pub async fn get_jwt_token_force(&mut self) -> Result<String> {
        let now = Utc::now().timestamp();

        let request_url = self.oauth_server.join("/refresh")?;

        debug!("Sending RefreshToken: {}", self.token_data.refresh_token);
        let response = self
            .client
            .post(request_url)
            .query(&vec![("token", self.token_data.refresh_token.clone())])
            .send()
            .await?;

        if response.status().is_success() {
            // todo: remove raw_response and add validate response;
            let response_text = response.text().await?;
            let response_json: Value = serde_json::from_str(&*response_text)?;
            let token = response_json["AccessToken"]
                .as_str()
                .ok_or_else(|| anyhow!("AccessToken missing in refresh response"))?
                .to_string();
            self.token_data.raw_response = Some(response_text);
            self.token_data.token = Some(token.clone());
            self.token_data.token_decoded = Some(self.decode_token(token)?);
            self.token_data.token_issued = now;

            // todo: move to init?
            self.parse_data_from_token();
        } else {
            self.token_data.token = None;
            self.token_data.token_decoded = None;
            self.token_data.token_issued = 0;
            self.token_data.raw_response = None;
            return Err(anyhow!("Failed to refresh JWT token: HTTP {}", response.status()));
        }

        self.token_data
            .raw_response
            .clone()
            .ok_or_else(|| anyhow!("JWT token refresh succeeded but raw response is missing"))
    }

    fn decode_token(&self, token: String) -> Result<Value> {
        let token_split = token.split(".");

        let token_data = token_split
            .skip(1)
            .next()
            .ok_or_else(|| anyhow!("Invalid JWT format: payload part missing"))?;
        let data_string = Self::decode_base64_with_padding(token_data)?;

        Ok(serde_json::from_str(&data_string)?)
    }

    fn parse_data_from_token(&mut self) {
        let portfolio_number = env::var("PORTFOLIO_NUMBER").ok();

        let Some(token_data) = self.token_data.token_decoded.clone() else {
            warn!("token_decoded is empty, skip parse_data_from_token");
            return;
        };
        self.user_data.accounts.clear();

        let Some(agreements_str) = token_data["agreements"].as_str() else {
            warn!("JWT token has no agreements field");
            return;
        };
        let Some(portfolios_str) = token_data["portfolios"].as_str() else {
            warn!("JWT token has no portfolios field");
            return;
        };
        let all_agreements = agreements_str.split(" ").collect::<Vec<&str>>();
        let all_portfolios = portfolios_str.split(" ").collect::<Vec<&str>>();

        let mut portfolio_id = 0;
        for (account_id, agreement) in all_agreements.iter().enumerate() {
            // Пробегаемся по всем договорам
            for portfolio in all_portfolios
                .clone()
                .into_iter()
                .skip(portfolio_id)
                .take(3)
            {
                // Пробегаемся по 3 - м портфелям каждого договора
                let (portfolio_type, exchanges, boards) = match portfolio.chars().next() {
                    // Паттерн-матчинг для первых символов
                    Some('D') => (
                        "securities",
                        self.exchanges.clone(),
                        vec![
                            "TQRD".to_string(),
                            "TQOY".to_string(),
                            "TQIF".to_string(),
                            "TQBR".to_string(),
                            "MTQR".to_string(),
                            "TQOB".to_string(),
                            "TQIR".to_string(),
                            "EQRP_INFO".to_string(),
                            "TQTF".to_string(),
                            "FQDE".to_string(),
                            "INDX".to_string(),
                            "TQOD".to_string(),
                            "FQBR".to_string(),
                            "TQCB".to_string(),
                            "TQPI".to_string(),
                            "TQBD".to_string(),
                        ],
                    ),
                    Some('G') => (
                        "fx",
                        vec![self.exchanges[0].clone()],
                        vec![
                            "CETS_SU".to_string(),
                            "INDXC".to_string(),
                            "CETS".to_string(),
                        ],
                    ),
                    Some('7')
                        if portfolio_number
                            .as_deref()
                            .map(|prefix| portfolio.starts_with(prefix))
                            .unwrap_or(true) =>
                    (
                        "derivatives",
                        vec![self.exchanges[0].clone()],
                        vec![
                            "SPBOPT".to_string(),
                            "OPTCOMDTY".to_string(),
                            "OPTSPOT".to_string(),
                            "SPBFUT".to_string(),
                            "OPTCURNCY".to_string(),
                            "RFUD".to_string(),
                            "ROPD".to_string(),
                        ],
                    ),
                    _ => {
                        warn!(
                            "Не определен тип счета для договора {}, портфеля {}",
                            agreement, portfolio
                        );
                        continue;
                    }
                };

                let user = Account {
                    account_id: account_id as i32,
                    agreement: agreement.to_string(),
                    portfolio: portfolio.to_string(),
                    portfolio_type: portfolio_type.to_string(),
                    exchanges,
                    boards,
                };

                self.user_data.accounts.push(user); // Добавляем договор/портфель/биржи/режимы торгов
            }

            portfolio_id += 3; // Смещаем на начальную позицию портфелей для следующего договора
        }
    }

    fn decode_base64_with_padding(encoded: &str) -> Result<String> {
        // Calculate the number of padding characters needed
        let padding_needed = (4 - encoded.len() % 4) % 4;

        // Create a new string with padding
        let padded_encoded = format!("{}{}", encoded, "=".repeat(padding_needed));

        Ok(String::from_utf8(
            general_purpose::STANDARD.decode(&padded_encoded)?,
        )?)
    }
}
