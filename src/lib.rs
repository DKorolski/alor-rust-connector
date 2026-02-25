use crate::structs::cws::CWS;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use futures_util::SinkExt;
use log::{debug, info};
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::Url;
use serde_json::{Error, Number, Value};
use std::error::Error as StdError;
use std::io;
use std::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Utf8Bytes;


pub mod dto {
    pub mod cws_dto;
}

mod helpers {
    pub mod servers;
}

pub mod structs {
    pub mod auth_client;

    pub mod api_client;

    pub mod token_data;

    pub mod user_data;

    pub mod history_data;

    pub mod ws;

    pub mod cws;

    pub mod dto;
}

use structs::api_client::*;
use structs::history_data::*;
use uuid::Uuid;
use structs::auth_client::AuthClient;

use anyhow::Result;

/// Formats the sum of two numbers as string.
pub struct AlorRust {
    pub client: ApiClient,
    pub auth_client: AuthClient,
    pub cws_client: CWS,
}

impl AlorRust {
    pub async fn new(
        refresh_token: &str,
        demo: bool,
        ws_callback: fn(&Value),
        cws_callback: fn(&Value),
    ) -> Result<Self> {
        info!("Initializing AlorRust, demo status: {demo}");

        let mut api = AlorRust {
            auth_client: AuthClient::new(refresh_token, demo)?,
            client: ApiClient::new(demo, ws_callback).await?,
            cws_client: CWS::new(refresh_token, demo, cws_callback).await?,
        };

        api.auth_client.get_jwt_token().await.unwrap();

        Ok(api)
    }

    // internal

    async fn get_headers_vec(&mut self) -> HeaderMap {
        let data_json: Value =
            serde_json::from_str(self.auth_client.get_jwt_token().await.unwrap().as_str()).unwrap();

        let mut headers = HeaderMap::new();

        headers.insert(
            "Content-Type",
            HeaderValue::from_str("application/json").unwrap(),
        );
        headers.insert(
            "Authorization",
            HeaderValue::from_str(
                format!("Bearer {}", data_json["AccessToken"].as_str().unwrap()).as_str(),
            )
            .unwrap(),
        );

        headers
    }

    async fn get_symbol_info_internal(
        &mut self,
        exchange: &str,
        symbol: &str,
        reload: bool,
    ) -> Option<Value> {
        if self
            .auth_client
            .symbols
            .get(format!("{}-{}", exchange, symbol).as_str())
            .is_none()
            || reload
        {
            self.get_symbol(exchange, symbol, None, "Simple")
                .await
                .unwrap();
        }

        self.auth_client
            .symbols
            .get(format!("{}-{}", exchange, symbol).as_str())
            .cloned()
    }

    async fn find_symbol_in_exchange(&mut self, symbol: &str) -> Option<String> {
        for exchange in self.auth_client.exchanges.clone().iter() {
            let si = self.get_symbol_info_internal(exchange, symbol, false).await; // Получаем информацию о тикере

            if let Some(data) = si {
                return Some(data.clone()["board"].as_str()?.to_string());
            }
        }

        None
    }

    async fn find_exchange_by_symbol(&mut self, board: &str, symbol: &str) -> Option<String> {
        for exchange in self.auth_client.exchanges.clone().iter() {
            let si = self.get_symbol_info_internal(exchange, symbol, false).await; // Получаем информацию о тикере

            if let Some(data) = si {
                if data.clone()["board"] == board {
                    return Some((*exchange.clone()).to_string());
                }
            }
        }

        None
    }

    async fn get_history_data_chunk(
        exchange: &str,
        symbol: &str,
        tf: i64,
        from: i64,
        to: i64,
        untraded: bool,
        format: &str,
        block_size: i64,
        headers: HeaderMap,
        client: reqwest::Client,
        url: Url,
    ) -> HistoryDataResponse {
        let response = client
            .get(url)
            .headers(headers)
            .query(&vec![
                ("exchange", exchange),
                ("symbol", symbol),
                ("tf", tf.to_string().as_str()),
                ("from", from.max(to - block_size).to_string().as_str()),
                ("to", to.to_string().as_str()),
                ("untraded", untraded.to_string().as_str()),
                ("format", format),
            ])
            .send()
            .await
            .unwrap();

        if response.status().is_success() {
            response.json::<HistoryDataResponse>().await.unwrap()
        } else {
            panic!("{:?}", response.text().await.unwrap());
        }
    }

    // api

    pub async fn get_positions(
        &mut self,
        portfolio: &str,
        exchange: &str,
        without_currency: bool,
        format: &str,
    ) -> Result<Value> {
        debug!("start get_positions");

        let response = self
            .client
            .client
            .get(
                self.client
                    .api_server
                    .join(format!("/md/v2/Clients/{}/{}/positions", exchange, portfolio).as_str())
                    .unwrap(),
            )
            .headers(self.get_headers_vec().await)
            .query(&vec![
                ("withoutCurrency", without_currency.to_string().as_str()),
                ("format", format),
            ])
            .send()
            .await;
        let response_json = self.client.validate_response(response.unwrap()).await?;

        Ok(response_json)
    }

    /// get_history(exchange: str, symbol: str, tf: int, seconds_from: int = 1, seconds_to: int = 32536799999, untraded: bool = false, format: str = "Simple", block_size: int = 2500000)
    /// --
    /// Получение свечей за определенный интервал
    ///
    /// :param str exchange: Биржа 'MOEX' или 'SPBX'
    /// :param str symbol: Тикер
    /// :param int tf: длительность таймфрейма
    /// :param int seconds_from: дата с которой нужно получить свечи
    /// :param int seconds_to: дата по которую нужно получить свечи
    /// :param bool untraded:
    /// :param str format: Формат принимаемых данных 'Simple', 'Slim', 'Heavy'
    /// :param int block_size: Размер одного запроса на биржу
    ///
    pub async fn get_history(
        &mut self,
        exchange: &str,
        symbol: &str,
        tf: i64,
        seconds_from: i64,
        seconds_to: i64,
        untraded: bool,
        format: &str,
        block_size: i64,
    ) -> Result<Value, Box<dyn StdError>> {
        debug!("start get_history");

        let exchange_string = exchange.to_string();
        let symbol_string = symbol.to_string();
        let format_string = format.to_string();
        let client = reqwest::Client::new();
        let url = self.client.api_server.join("/md/v2/history").unwrap();
        let headers = self.get_headers_vec().await;

        let (tx, rx) = mpsc::channel();

        let mut result_data = HistoryDataResponse {
            history: vec![],
            next: None,
            prev: None,
        };

        let from: i64 = seconds_from;
        let to: i64 = seconds_to;

        let correct_from: i64;
        let mut correct_to: i64;
        if from + block_size >= to {
            correct_from = from;
            correct_to = to;
        } else {
            correct_from = AlorRust::get_history_data_chunk(
                exchange_string.as_str(),
                symbol_string.as_str(),
                tf,
                from,
                from + 10,
                untraded,
                format_string.as_str(),
                block_size,
                headers.clone(),
                client.clone(),
                url.clone(),
            )
            .await
            .next
            .unwrap();

            correct_to = AlorRust::get_history_data_chunk(
                exchange_string.as_str(),
                symbol_string.as_str(),
                tf,
                to - 10,
                to,
                untraded,
                format_string.as_str(),
                block_size,
                headers.clone(),
                client.clone(),
                url.clone(),
            )
            .await
            .prev
            .unwrap();
        }

        let mut tasks: Vec<JoinHandle<Result<HistoryDataResponse, Error>>> = Vec::new();
        loop {
            let exchange_string_copy = exchange_string.clone();
            let symbol_string_copy = symbol_string.clone();
            let format_string_copy = format_string.clone();
            let cloned_client = client.clone();
            let cloned_url = url.clone();
            let cloned_headers = headers.clone();

            tasks.push(tokio::spawn(async move {
                let data = AlorRust::get_history_data_chunk(
                    &exchange_string_copy,
                    &symbol_string_copy,
                    tf,
                    correct_from,
                    correct_to,
                    untraded,
                    &format_string_copy,
                    block_size,
                    cloned_headers.clone(),
                    cloned_client.clone(),
                    cloned_url.clone(),
                )
                .await;

                Ok(data)
            }));

            correct_to = correct_to - block_size;
            if correct_to <= 0 || correct_to < correct_from {
                break;
            }
        }

        for task in tasks {
            let mut result = task.await.unwrap().unwrap();

            result_data.history.append(&mut result.history);
        }
        result_data.remove_duplicate_histories();
        tx.send(result_data).unwrap();

        let result_data = rx.recv().unwrap();

        Ok(serde_json::to_value(result_data)?)
    }

    /// get_symbol(exchange, symbol, instrument_group=None, format="Simple")
    /// --
    /// Получение информации о выбранном финансовом инструменте
    ///
    /// :param str exchange: Биржа 'MOEX' или 'SPBX'
    /// :param str symbol: Тикер
    /// :param str instrument_group: Код режима торгов
    /// :param str format: Формат принимаемых данных 'Simple', 'Slim', 'Heavy'
    ///
    pub async fn get_symbol(
        &mut self,
        exchange: &str,
        symbol: &str,
        instrument_group: Option<String>,
        format: &str,
    ) -> Result<Value, Box<dyn StdError>> {
        debug!("start get symbol");
        let mut params = vec![("format", format)];

        if let Some(ref value) = instrument_group {
            params.push(("instrumentGroup", value.as_str()));
        };

        let response = self
            .client
            .client
            .get(
                self.client
                    .api_server
                    .join(format!("/md/v2/Securities/{}/{}", exchange, symbol).as_str())
                    .unwrap(),
            )
            .headers(self.get_headers_vec().await)
            .query(&params.clone())
            .send()
            .await;
        let mut response_json = self.client.validate_response(response.unwrap()).await?;

        let decimals = ((1.0 / response_json["minstep"].as_f64().unwrap()).log10() + 0.99) as i64;
        response_json["decimals"] = Value::Number(Number::from(decimals));

        self.auth_client
            .symbols
            .insert(format!("{}-{}", exchange, symbol), response_json.clone());

        Ok(response_json)
    }

    pub async fn get_symbol_info(
        &mut self,
        exchange: &str,
        symbol: &str,
        reload: bool,
    ) -> Result<Option<Value>, Box<dyn StdError>> {
        debug!("start get symbol info");

        if self
            .auth_client
            .symbols
            .get(format!("{}-{}", exchange, symbol).as_str())
            .is_none()
            || reload
        {
            self.get_symbol(exchange, symbol, None, "Simple").await?;
        }

        Ok(self
            .get_symbol_info_internal(exchange, symbol, reload)
            .await)
    }

    pub async fn get_server_time(&mut self) -> Result<u64, Box<dyn StdError>> {
        debug!("start get server time");

        let response = self
            .client
            .client
            .get(self.client.api_server.join("/md/v2/time").unwrap())
            .headers(self.get_headers_vec().await)
            .send()
            .await;
        let response_json = self.client.validate_response(response.unwrap()).await?;

        Ok(response_json.as_u64().unwrap())
    }

    // websocket methods

    /// subscribe_bars(self)
    /// --
    /// Подписка на историю цен (свечи) для выбранных биржи и финансового инструмента
    ///
    /// :param str exchange: Биржа 'MOEX' или 'SPBX'
    /// :param str symbol: Тикер
    /// :param tf: Длительность временнОго интервала в секундах или код ("D" - дни, "W" - недели, "M" - месяцы, "Y" - годы)
    /// :param int seconds_from: Дата и время UTC в секундах для первого запрашиваемого бара
    /// :param bool skip_history: Флаг отсеивания исторических данных: True — отображать только новые данные, False — отображать в том числе данные из истории
    /// :param int frequency: Максимальная частота отдачи данных сервером в миллисекундах
    /// :param str format: Формат принимаемых данных 'Simple', 'Slim', 'Heavy'
    /// :return: Уникальный идентификатор подписки
    pub async fn subscribe_bars(
        &mut self,
        exchange: &str,
        symbol: &str,
        tf: u32,
        seconds_from: u64,
        skip_history: bool,
        frequency: i32,
        format: &str,
    ) -> Result<Uuid, Box<dyn StdError>> {
        let data_json: Value = serde_json::from_str(self.auth_client.get_jwt_token().await?.as_str())?;
        let auth_token = data_json
            .get("AccessToken")
            .and_then(|v| v.as_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "AccessToken missing in JWT response"))?
            .to_string();

        let mut request_body = Value::Object(serde_json::Map::new());

        let subscribe_guid = Uuid::new_v4();
        if let Some(body) = request_body.as_object_mut() {
            body.insert("opcode".to_string(), Value::from("BarsGetAndSubscribe"));
            body.insert("exchange".to_string(), Value::from(exchange));
            body.insert("code".to_string(), Value::from(symbol));
            body.insert("tf".to_string(), Value::Number(Number::from(tf)));
            body.insert(
                "from".to_string(),
                Value::Number(Number::from(seconds_from)),
            );
            body.insert("delayed".to_string(), Value::from(false)); // Convert false to Value
            body.insert("skipHistory".to_string(), Value::from(skip_history));
            body.insert("frequency".to_string(), Value::from(frequency));
            body.insert("format".to_string(), Value::from(format));
            body.insert("prev".to_string(), Value::from(None::<i32>));
            body.insert("token".to_string(), Value::from(auth_token));
            body.insert("guid".to_string(), Value::from(subscribe_guid.to_string()));
        }

        let message = tokio_tungstenite::tungstenite::Message::Text(Utf8Bytes::from(
            request_body.to_string(),
        ));
        self.client
            .socket_client
            .write_stream
            .send(message)
            .await
            .map_err(|e| -> Box<dyn StdError> { Box::new(e) })?;

        Ok(subscribe_guid)
    }

    /// Подписка на статусы заявок через market-data WS (`OrdersGetAndSubscribeV2`).
    /// Источник фактического идентификатора заявки в событиях статуса: `data.id`.
    pub async fn subscribe_orders_statuses_v2(
        &mut self,
        exchange: &str,
        portfolio: &str,
        order_statuses: Option<Vec<String>>,
        skip_history: bool,
        frequency: i32,
        format: &str,
    ) -> Result<Uuid, Box<dyn StdError>> {
        let data_json: Value = serde_json::from_str(self.auth_client.get_jwt_token().await?.as_str())?;
        let auth_token = data_json
            .get("AccessToken")
            .and_then(|v| v.as_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "AccessToken missing in JWT response"))?
            .to_string();

        let mut request_body = Value::Object(serde_json::Map::new());
        let subscribe_guid = Uuid::new_v4();

        if let Some(body) = request_body.as_object_mut() {
            body.insert(
                "opcode".to_string(),
                Value::from("OrdersGetAndSubscribeV2"),
            );
            body.insert("exchange".to_string(), Value::from(exchange));
            body.insert("portfolio".to_string(), Value::from(portfolio));
            if let Some(statuses) = order_statuses {
                body.insert(
                    "orderStatuses".to_string(),
                    Value::Array(statuses.into_iter().map(Value::from).collect()),
                );
            }
            body.insert("skipHistory".to_string(), Value::from(skip_history));
            body.insert("frequency".to_string(), Value::from(frequency));
            body.insert("format".to_string(), Value::from(format));
            body.insert("guid".to_string(), Value::from(subscribe_guid.to_string()));
            body.insert("token".to_string(), Value::from(auth_token));
        }

        let message = tokio_tungstenite::tungstenite::Message::Text(Utf8Bytes::from(
            request_body.to_string(),
        ));
        self.client
            .socket_client
            .write_stream
            .send(message)
            .await
            .map_err(|e| -> Box<dyn StdError> { Box::new(e) })?;

        Ok(subscribe_guid)
    }

    // convert

    pub async fn dataname_to_board_symbol(
        &mut self,
        dataname: &str,
    ) -> Result<Vec<String>, Box<dyn StdError>> {
        debug!("parse dataname_to_board_symbol");
        let symbol_parts: Vec<&str> = dataname.split('.').collect();

        let mut board: String;
        let symbol: String;

        if symbol_parts.len() >= 2 {
            // Если тикер задан в формате <Код режима торгов>.<Код тикера>
            board = symbol_parts[0].to_string(); // Код режима торгов
            symbol = symbol_parts[1..].join("."); // Код тикера
        } else {
            symbol = dataname.to_string();
            board = self
                .find_symbol_in_exchange(symbol.as_str())
                .await
                .unwrap()
                .to_string();
        }

        if board.clone() == "SPBFUT" {
            // Для фьючерсов
            board = "RFUD".to_string(); // Меняем код режима торгов на принятое в Алоре
        } else if board.clone() == "SPBOPT" {
            // Для опционов
            board = "ROPD".to_string(); // Меняем код режима торгов на принятое в Алоре
        }

        Ok(vec![board, symbol])
    }

    pub async fn get_exchange(
        &mut self,
        board: &str,
        symbol: &str,
    ) -> Result<String, Box<dyn StdError>> {
        debug!("get exchange by board: {board} and symbol: {symbol}");
        let mut using_board = board.to_string();

        if using_board == "SPBFUT" {
            // Для фьючерсов
            using_board = "RFUD".to_string(); // Меняем код режима торгов на принятое в Алоре
        } else if using_board == "SPBOPT" {
            // Для опционов
            using_board = "ROPD".to_string(); // Меняем код режима торгов на принятое в Алоре
        }
        let exchange = self.find_exchange_by_symbol(using_board.as_str(), symbol);

        Ok(exchange.await.unwrap())
    }

    pub fn utc_timestamp_to_msk_datetime(
        &self,
        timestamp: i64,
    ) -> Result<DateTime<Utc>, Box<dyn StdError>> {
        // Convert Unix timestamp to Utc DateTime
        Ok(DateTime::from_timestamp(timestamp, 0).unwrap())
    }

    /// msk_datetime_to_utc_timestamp(date)
    /// --
    /// Перевод московского времени в кол-во секунд, прошедших с 01.01.1970 00:00 UTC
    ///
    /// :param datetime dt: Московское время
    /// :return: Кол-во секунд, прошедших с 01.01.1970 00:00 UTC
    ///
    pub fn msk_datetime_to_utc_timestamp(
        &self,
        date: DateTime<Utc>,
    ) -> Result<u64, Box<dyn StdError>> {
        let date_with_tz: DateTime<Tz> = date.with_timezone(&"Europe/Moscow".parse::<Tz>()?);

        Ok(date_with_tz.timestamp() as u64)
    }

    // class methods

    pub fn get_account(
        &mut self,
        board: &str,
        account_id: i32,
    ) -> Result<Option<Value>, Box<dyn StdError>> {
        let account = self.auth_client.user_data.find_account(account_id, board);

        if let Some(account) = account {
            return Ok(Some(serde_json::to_value(account)?));
        };

        Ok(None)
    }

    pub fn accounts(&self) -> Result<Vec<Value>, Box<dyn StdError>> {
        let mut result = vec![];

        for account in self.auth_client.user_data.accounts.iter() {
            let _ = result.push(serde_json::to_value(account).unwrap());
        }

        Ok(result)
    }
}

pub fn init_logger(level: &str) {
    std::env::set_var("RUST_LOG", level);
    let _ = env_logger::try_init();
}
