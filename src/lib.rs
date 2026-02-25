use crate::structs::cws::CWS;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use futures_util::SinkExt;
use log::{debug, info};
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::Url;
use serde_json::{Number, Value};
use std::io;
use tokio::task::JoinHandle;
use tokio::sync::broadcast;
use tokio::time::{timeout, Duration};
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

use anyhow::{anyhow, Result};

/// Formats the sum of two numbers as string.
pub struct AlorRust {
    pub client: ApiClient,
    pub auth_client: AuthClient,
    pub cws_client: CWS,
}

#[derive(Debug, Clone)]
pub struct CreateLimitOrderFlowResult {
    pub request_guid: String,
    pub cws_ack: Value,
    pub ws_status_event: Value,
    pub order_id: String,
}

#[derive(Debug, Clone)]
pub struct DeleteLimitOrderFlowResult {
    pub request_guid: String,
    pub cws_ack: Value,
    pub ws_status_event: Value,
    pub order_id: String,
}

#[derive(Debug, Clone)]
pub struct UpdateLimitOrderFlowResult {
    pub request_guid: String,
    pub cws_ack: Value,
    pub old_order_status_event: Option<Value>,
    pub new_order_status_event: Value,
    pub old_order_id: String,
    pub new_order_id: String,
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

        api.auth_client
            .get_jwt_token()
            .await?;

        Ok(api)
    }

    // internal

    async fn get_headers_vec(&mut self) -> anyhow::Result<HeaderMap> {
        let jwt_raw = self
            .auth_client
            .get_jwt_token()
            .await?;
        let data_json: Value = serde_json::from_str(jwt_raw.as_str())?;

        let mut headers = HeaderMap::new();

        headers.insert(
            "Content-Type",
            HeaderValue::from_str("application/json")?,
        );
        let access_token = data_json
            .get("AccessToken")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("AccessToken missing in JWT response"))?;
        headers.insert(
            "Authorization",
            HeaderValue::from_str(format!("Bearer {}", access_token).as_str())?,
        );

        Ok(headers)
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
            if self.get_symbol(exchange, symbol, None, "Simple").await.is_err() {
                return None;
            }
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
    ) -> anyhow::Result<HistoryDataResponse> {
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
            .await?;

        if response.status().is_success() {
            Ok(response.json::<HistoryDataResponse>().await?)
        } else {
            Err(anyhow!(response.text().await.unwrap_or_else(|_| "history request failed".to_string())))
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
                    ?,
            )
            .headers(self.get_headers_vec().await?)
            .query(&vec![
                ("withoutCurrency", without_currency.to_string().as_str()),
                ("format", format),
            ])
            .send()
            .await;
        let response_json = self.client.validate_response(response?).await?;

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
    ) -> Result<Value> {
        debug!("start get_history");

        let exchange_string = exchange.to_string();
        let symbol_string = symbol.to_string();
        let format_string = format.to_string();
        let client = reqwest::Client::new();
        let url = self.client.api_server.join("/md/v2/history")?;
        let headers = self.get_headers_vec().await?;

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
            .map_err(anyhow::Error::msg)?
            .next
            .ok_or_else(|| anyhow!("No next history cursor"))?;

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
            .map_err(anyhow::Error::msg)?
            .prev
            .ok_or_else(|| anyhow!("No prev history cursor"))?;
        }

        let mut tasks: Vec<JoinHandle<anyhow::Result<HistoryDataResponse>>> = Vec::new();
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
                .await?;

                Ok(data)
            }));

            correct_to = correct_to - block_size;
            if correct_to <= 0 || correct_to < correct_from {
                break;
            }
        }

        for task in tasks {
            let mut result = task
                .await
                .map_err(anyhow::Error::msg)?
                .map_err(anyhow::Error::msg)?;

            result_data.history.append(&mut result.history);
        }
        result_data.remove_duplicate_histories();

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
    ) -> Result<Value> {
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
                    ?,
            )
            .headers(self.get_headers_vec().await?)
            .query(&params.clone())
            .send()
            .await;
        let mut response_json = self.client.validate_response(response?).await?;

        let minstep = response_json["minstep"]
            .as_f64()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "minstep missing or invalid"))?;
        let decimals = ((1.0 / minstep).log10() + 0.99) as i64;
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
    ) -> Result<Option<Value>> {
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

    pub async fn get_server_time(&mut self) -> Result<u64> {
        debug!("start get server time");

        let response = self
            .client
            .client
            .get(self.client.api_server.join("/md/v2/time")?)
            .headers(self.get_headers_vec().await?)
            .send()
            .await;
        let response_json = self.client.validate_response(response?).await?;

        response_json
            .as_u64()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "server time is not u64").into())
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
    ) -> Result<Uuid> {
        let jwt_raw = self
            .auth_client
            .get_jwt_token()
            .await?;
        let data_json: Value = serde_json::from_str(jwt_raw.as_str())?;
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
            .await?;

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
    ) -> anyhow::Result<Uuid> {
        let jwt_raw = self
            .auth_client
            .get_jwt_token()
            .await?;
        let data_json: Value = serde_json::from_str(jwt_raw.as_str())?;
        let auth_token = data_json
            .get("AccessToken")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("AccessToken missing in JWT response"))?
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
            .await?;

        Ok(subscribe_guid)
    }

    pub fn subscribe_ws_events(&self) -> broadcast::Receiver<Value> {
        self.client.socket_client.subscribe_events()
    }

    pub fn subscribe_cws_events(&self) -> broadcast::Receiver<Value> {
        self.cws_client.subscribe_events()
    }

    pub fn cws_request_guid(event: &Value) -> Option<String> {
        event
            .get("requestGuid")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    pub fn cws_order_number(event: &Value) -> Option<String> {
        event.get("orderNumber")
            .and_then(|v| {
                if v.is_string() {
                    v.as_str().map(|s| s.to_string())
                } else {
                    Some(v.to_string())
                }
            })
    }

    pub fn cws_http_code(event: &Value) -> Option<u64> {
        event.get("httpCode").and_then(|v| v.as_u64())
    }

    pub fn ws_event_guid(event: &Value) -> Option<String> {
        event
            .get("guid")
            .or_else(|| event.get("requestGuid"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    pub fn ws_order_status_id(event: &Value) -> Option<String> {
        event.get("data")
            .and_then(|d| d.get("id"))
            .and_then(|v| {
                if v.is_string() {
                    v.as_str().map(|s| s.to_string())
                } else {
                    Some(v.to_string())
                }
            })
    }

    pub fn ws_order_status(event: &Value) -> Option<String> {
        event.get("data")
            .and_then(|d| d.get("status"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_lowercase())
    }

    pub async fn wait_cws_event_by_request_guid(
        rx: &mut broadcast::Receiver<Value>,
        request_guid: &str,
        timeout_duration: Duration,
    ) -> anyhow::Result<Value> {
        let fut = async {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if Self::cws_request_guid(&event).as_deref() == Some(request_guid) {
                            return Ok(event);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(anyhow!("CWS event stream closed"));
                    }
                }
            }
        };

        timeout(timeout_duration, fut)
            .await
            .map_err(|_| anyhow!("Timed out waiting for CWS event"))?
    }

    pub async fn wait_ws_event_by_guid(
        rx: &mut broadcast::Receiver<Value>,
        guid: &str,
        timeout_duration: Duration,
    ) -> anyhow::Result<Value> {
        let fut = async {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if Self::ws_event_guid(&event).as_deref() == Some(guid) {
                            return Ok(event);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(anyhow!("WS event stream closed"));
                    }
                }
            }
        };

        timeout(timeout_duration, fut)
            .await
            .map_err(|_| anyhow!("Timed out waiting for WS event"))?
    }

    pub async fn wait_ws_order_status_by_id(
        rx: &mut broadcast::Receiver<Value>,
        order_id: &str,
        timeout_duration: Duration,
    ) -> anyhow::Result<Value> {
        let fut = async {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if Self::ws_order_status_id(&event).as_deref() == Some(order_id) {
                            return Ok(event);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(anyhow!("WS event stream closed"));
                    }
                }
            }
        };

        timeout(timeout_duration, fut)
            .await
            .map_err(|_| anyhow!("Timed out waiting for WS order status event"))?
    }

    pub async fn wait_ws_order_status_by_id_and_predicate(
        rx: &mut broadcast::Receiver<Value>,
        order_id: &str,
        timeout_duration: Duration,
        predicate: fn(&str) -> bool,
    ) -> anyhow::Result<Value> {
        let fut = async {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if Self::ws_order_status_id(&event).as_deref() != Some(order_id) {
                            continue;
                        }
                        let Some(status) = Self::ws_order_status(&event) else {
                            continue;
                        };
                        if !predicate(&status) {
                            continue;
                        }
                        return Ok(event);
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(anyhow!("WS event stream closed"));
                    }
                }
            }
        };

        timeout(timeout_duration, fut)
            .await
            .map_err(|_| anyhow!("Timed out waiting for WS order status event by predicate"))?
    }

    pub async fn wait_ws_order_status_event(
        rx: &mut broadcast::Receiver<Value>,
        subscribe_guid: &str,
        portfolio: &str,
        symbol: &str,
        timeout_duration: Duration,
    ) -> anyhow::Result<Value> {
        let fut = async {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if Self::ws_event_guid(&event).as_deref() != Some(subscribe_guid) {
                            continue;
                        }

                        let data = match event.get("data") {
                            Some(d) => d,
                            None => continue,
                        };

                        let ev_portfolio = data.get("portfolio").and_then(|v| v.as_str());
                        let ev_symbol = data.get("symbol").and_then(|v| v.as_str());
                        if ev_portfolio != Some(portfolio) || ev_symbol != Some(symbol) {
                            continue;
                        }

                        return Ok(event);
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(anyhow!("WS event stream closed"));
                    }
                }
            }
        };

        timeout(timeout_duration, fut)
            .await
            .map_err(|_| anyhow!("Timed out waiting for WS order status event by subscription"))?
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_limit_order_and_wait_status_id(
        &mut self,
        cws_rx: &mut broadcast::Receiver<Value>,
        ws_rx: &mut broadcast::Receiver<Value>,
        ws_subscribe_guid: &str,
        side: dto::cws_dto::order_common::OrderSide,
        quantity: i32,
        price: f64,
        symbol: &str,
        exchange: &str,
        instrument_group: Option<&str>,
        portfolio: &str,
        comment: Option<String>,
        time_in_force: Option<dto::cws_dto::order_common::TimeInForce>,
        allow_margin: Option<bool>,
        iceberg_fixed: Option<i32>,
        iceberg_variance: Option<i32>,
        check_duplicates: Option<bool>,
        timeout_duration: Duration,
    ) -> anyhow::Result<CreateLimitOrderFlowResult> {
        let request_guid = self
            .cws_client
            .create_limit_order(
                side,
                quantity,
                price,
                symbol,
                exchange,
                instrument_group,
                portfolio,
                comment,
                time_in_force,
                allow_margin,
                iceberg_fixed,
                iceberg_variance,
                check_duplicates,
            )
            .await?;

        let cws_ack =
            Self::wait_cws_event_by_request_guid(cws_rx, &request_guid, timeout_duration).await?;

        if let Some(code) = Self::cws_http_code(&cws_ack) {
            if code != 200 {
                return Err(anyhow!("create_limit_order failed, cws ack: {}", cws_ack));
            }
        }

        let ws_status_event = if let Some(order_id) = Self::cws_order_number(&cws_ack) {
            Self::wait_ws_order_status_by_id(ws_rx, &order_id, timeout_duration).await?
        } else {
            Self::wait_ws_order_status_event(
                ws_rx,
                ws_subscribe_guid,
                portfolio,
                symbol,
                timeout_duration,
            )
            .await
            .map_err(|e| anyhow!("No order id in CWS ack and WS status not received. cws ack: {} ; err: {}", cws_ack, e))?
        };

        let order_id = Self::ws_order_status_id(&ws_status_event)
            .or_else(|| Self::cws_order_number(&cws_ack))
            .ok_or_else(|| anyhow!("No order id in WS status event or CWS ack"))?;

        Ok(CreateLimitOrderFlowResult {
            request_guid,
            cws_ack,
            ws_status_event,
            order_id,
        })
    }

    pub async fn delete_limit_order_and_wait_status(
        &mut self,
        cws_rx: &mut broadcast::Receiver<Value>,
        ws_rx: &mut broadcast::Receiver<Value>,
        order_id: &str,
        exchange: &str,
        portfolio: &str,
        check_duplicates: Option<bool>,
        timeout_duration: Duration,
    ) -> anyhow::Result<DeleteLimitOrderFlowResult> {
        let request_guid = self
            .cws_client
            .delete_limit_order(order_id, exchange, portfolio, check_duplicates)
            .await?;

        let cws_ack =
            Self::wait_cws_event_by_request_guid(cws_rx, &request_guid, timeout_duration).await?;

        if let Some(code) = Self::cws_http_code(&cws_ack) {
            if code != 200 {
                return Err(anyhow!("delete_limit_order failed, cws ack: {}", cws_ack));
            }
        }

        let ws_status_event =
            Self::wait_ws_order_status_by_id(ws_rx, order_id, timeout_duration).await?;

        let resolved_order_id = Self::ws_order_status_id(&ws_status_event)
            .or_else(|| Self::cws_order_number(&cws_ack))
            .unwrap_or_else(|| order_id.to_string());

        Ok(DeleteLimitOrderFlowResult {
            request_guid,
            cws_ack,
            ws_status_event,
            order_id: resolved_order_id,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_limit_order_and_wait_status(
        &mut self,
        cws_rx: &mut broadcast::Receiver<Value>,
        ws_rx: &mut broadcast::Receiver<Value>,
        ws_subscribe_guid: &str,
        old_order_id: &str,
        side: dto::cws_dto::order_common::OrderSide,
        quantity: i32,
        price: f64,
        symbol: &str,
        exchange: &str,
        instrument_group: Option<&str>,
        portfolio: &str,
        comment: Option<String>,
        allow_margin: Option<bool>,
        iceberg_fixed: Option<i32>,
        check_duplicates: Option<bool>,
        timeout_duration: Duration,
    ) -> anyhow::Result<UpdateLimitOrderFlowResult> {
        let request_guid = self
            .cws_client
            .update_limit_order(
                old_order_id,
                side,
                quantity,
                price,
                symbol,
                exchange,
                instrument_group,
                portfolio,
                comment,
                allow_margin,
                iceberg_fixed,
                check_duplicates,
            )
            .await?;

        let cws_ack =
            Self::wait_cws_event_by_request_guid(cws_rx, &request_guid, timeout_duration).await?;

        if let Some(code) = Self::cws_http_code(&cws_ack) {
            if code != 200 {
                return Err(anyhow!("update_limit_order failed, cws ack: {}", cws_ack));
            }
        }

        let ack_order_id = Self::cws_order_number(&cws_ack);

        // На некоторых рынках update реализуется как cancel old + create new.
        // Если сервер вернул новый order id и он отличается, ждём две WS записи:
        // 1) old -> canceled, 2) new -> working/любой статус.
        if let Some(new_order_id) = ack_order_id.clone() {
            if new_order_id != old_order_id {
                let old_order_status_event = Self::wait_ws_order_status_by_id_and_predicate(
                    ws_rx,
                    old_order_id,
                    timeout_duration,
                    |s| s == "canceled" || s == "cancelled",
                )
                .await
                .ok();

                let new_order_status_event =
                    Self::wait_ws_order_status_by_id(ws_rx, &new_order_id, timeout_duration)
                        .await?;

                return Ok(UpdateLimitOrderFlowResult {
                    request_guid,
                    cws_ack,
                    old_order_status_event,
                    new_order_status_event,
                    old_order_id: old_order_id.to_string(),
                    new_order_id,
                });
            }
        }

        // Fallback: либо id не изменился, либо в CWS ack id не пришёл.
        let new_order_status_event = if let Some(same_id) = ack_order_id.clone() {
            Self::wait_ws_order_status_by_id(ws_rx, &same_id, timeout_duration).await?
        } else {
            Self::wait_ws_order_status_event(
                ws_rx,
                ws_subscribe_guid,
                portfolio,
                symbol,
                timeout_duration,
            )
            .await?
        };

        let new_order_id = Self::ws_order_status_id(&new_order_status_event)
            .or(ack_order_id)
            .unwrap_or_else(|| old_order_id.to_string());

        Ok(UpdateLimitOrderFlowResult {
            request_guid,
            cws_ack,
            old_order_status_event: None,
            new_order_status_event,
            old_order_id: old_order_id.to_string(),
            new_order_id,
        })
    }

    // convert

    pub async fn dataname_to_board_symbol(
        &mut self,
        dataname: &str,
    ) -> Result<Vec<String>> {
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
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Symbol not found in known exchanges"))?
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
    ) -> Result<String> {
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

        exchange
            .await
            .ok_or_else(|| anyhow!("Exchange not found for board/symbol"))
    }

    pub fn utc_timestamp_to_msk_datetime(
        &self,
        timestamp: i64,
    ) -> Result<DateTime<Utc>> {
        // Convert Unix timestamp to Utc DateTime
        DateTime::from_timestamp(timestamp, 0)
            .ok_or_else(|| anyhow!("Invalid unix timestamp"))
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
    ) -> Result<u64> {
        let date_with_tz: DateTime<Tz> = date.with_timezone(&"Europe/Moscow".parse::<Tz>()?);

        Ok(date_with_tz.timestamp() as u64)
    }

    // class methods

    pub fn get_account(
        &mut self,
        board: &str,
        account_id: i32,
    ) -> Result<Option<Value>> {
        let account = self.auth_client.user_data.find_account(account_id, board);

        if let Some(account) = account {
            return Ok(Some(serde_json::to_value(account)?));
        };

        Ok(None)
    }

    pub fn accounts(&self) -> Result<Vec<Value>> {
        let mut result = vec![];

        for account in self.auth_client.user_data.accounts.iter() {
            result.push(serde_json::to_value(account)?);
        }

        Ok(result)
    }
}

pub fn init_logger(level: &str) {
    std::env::set_var("RUST_LOG", level);
    let _ = env_logger::try_init();
}

#[cfg(test)]
mod tests {
    use super::helpers::servers::get_server_url;
    use super::AlorRust;
    use serde_json::json;

    #[test]
    fn ws_event_guid_reads_guid() {
        let event = json!({ "guid": "abc-123" });
        assert_eq!(AlorRust::ws_event_guid(&event).as_deref(), Some("abc-123"));
    }

    #[test]
    fn ws_event_guid_falls_back_to_request_guid() {
        let event = json!({ "requestGuid": "req-1" });
        assert_eq!(AlorRust::ws_event_guid(&event).as_deref(), Some("req-1"));
    }

    #[test]
    fn cws_order_number_reads_string_and_number() {
        let s = json!({ "orderNumber": "42" });
        let n = json!({ "orderNumber": 42 });
        assert_eq!(AlorRust::cws_order_number(&s).as_deref(), Some("42"));
        assert_eq!(AlorRust::cws_order_number(&n).as_deref(), Some("42"));
    }

    #[test]
    fn ws_order_status_extracts_id_and_status() {
        let event = json!({
            "data": {
                "id": "2033125999100562015",
                "status": "working"
            }
        });
        assert_eq!(
            AlorRust::ws_order_status_id(&event).as_deref(),
            Some("2033125999100562015")
        );
        assert_eq!(AlorRust::ws_order_status(&event).as_deref(), Some("working"));
    }

    #[test]
    fn cws_http_code_extracts_value() {
        let event = json!({ "httpCode": 400 });
        assert_eq!(AlorRust::cws_http_code(&event), Some(400));
    }

    #[test]
    fn server_urls_switch_between_demo_and_prod() {
        assert_eq!(
            get_server_url("api_server", false).unwrap(),
            "https://api.alor.ru"
        );
        assert_eq!(
            get_server_url("api_server", true).unwrap(),
            "https://apidev.alor.ru"
        );
    }
}
