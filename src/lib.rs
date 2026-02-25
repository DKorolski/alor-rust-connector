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
    pub mod events;
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
pub use dto::events::{CwsAckEvent, WsOrderStatusEvent, WsSubscribeAckEvent};

use anyhow::{anyhow, Result};

fn noop_json_callback(_: &Value) {}

#[derive(Clone, Copy)]
pub struct AlorConfig {
    pub demo: bool,
    pub ws_callback: fn(&Value),
    pub cws_callback: fn(&Value),
    pub enable_ws: bool,
    pub enable_cws: bool,
    pub preload_jwt: bool,
}

impl Default for AlorConfig {
    fn default() -> Self {
        Self {
            demo: false,
            ws_callback: noop_json_callback,
            cws_callback: noop_json_callback,
            enable_ws: true,
            enable_cws: true,
            preload_jwt: true,
        }
    }
}

pub struct AlorClientBuilder {
    refresh_token: String,
    config: AlorConfig,
}

impl AlorClientBuilder {
    pub fn new(refresh_token: impl Into<String>) -> Self {
        Self {
            refresh_token: refresh_token.into(),
            config: AlorConfig::default(),
        }
    }

    pub fn demo(mut self, demo: bool) -> Self {
        self.config.demo = demo;
        self
    }

    pub fn ws_callback(mut self, callback: fn(&Value)) -> Self {
        self.config.ws_callback = callback;
        self
    }

    pub fn cws_callback(mut self, callback: fn(&Value)) -> Self {
        self.config.cws_callback = callback;
        self
    }

    pub fn enable_ws(mut self, enabled: bool) -> Self {
        self.config.enable_ws = enabled;
        self
    }

    pub fn enable_cws(mut self, enabled: bool) -> Self {
        self.config.enable_cws = enabled;
        self
    }

    pub fn preload_jwt(mut self, enabled: bool) -> Self {
        self.config.preload_jwt = enabled;
        self
    }

    pub async fn build(self) -> Result<AlorRust> {
        AlorRust::from_config(&self.refresh_token, self.config).await
    }
}

/// Formats the sum of two numbers as string.
pub struct AlorRust {
    pub client: ApiClient,
    pub auth_client: AuthClient,
    pub cws_client: CWS,
}

#[derive(Debug, Clone)]
pub struct CreateLimitOrderFlowResult {
    pub request_guid: String,
    pub cws_ack: CwsAckEvent,
    pub ws_status_event: WsOrderStatusEvent,
    pub order_id: String,
}

#[derive(Debug, Clone)]
pub struct DeleteLimitOrderFlowResult {
    pub request_guid: String,
    pub cws_ack: CwsAckEvent,
    pub ws_status_event: WsOrderStatusEvent,
    pub order_id: String,
}

#[derive(Debug, Clone)]
pub struct UpdateLimitOrderFlowResult {
    pub request_guid: String,
    pub cws_ack: CwsAckEvent,
    pub old_order_status_event: Option<WsOrderStatusEvent>,
    pub new_order_status_event: WsOrderStatusEvent,
    pub old_order_id: String,
    pub new_order_id: String,
}

impl AlorRust {
    pub fn builder(refresh_token: impl Into<String>) -> AlorClientBuilder {
        AlorClientBuilder::new(refresh_token)
    }

    pub async fn new_default_callbacks(refresh_token: &str, demo: bool) -> Result<Self> {
        Self::from_config(
            refresh_token,
            AlorConfig {
                demo,
                ..AlorConfig::default()
            },
        )
        .await
    }

    pub async fn from_config(refresh_token: &str, config: AlorConfig) -> Result<Self> {
        if !config.enable_ws || !config.enable_cws {
            return Err(anyhow!(
                "Config flags enable_ws/enable_cws are not supported yet (REST-only/lazy init planned in Iteration 3)"
            ));
        }

        info!("Initializing AlorRust, demo status: {}", config.demo);

        let mut api = AlorRust {
            auth_client: AuthClient::new(refresh_token, config.demo)?,
            client: ApiClient::new(config.demo, config.ws_callback).await?,
            cws_client: CWS::new(refresh_token, config.demo, config.cws_callback).await?,
        };

        if config.preload_jwt {
            api.auth_client.get_jwt_token().await?;
        }

        Ok(api)
    }

    pub async fn new(
        refresh_token: &str,
        demo: bool,
        ws_callback: fn(&Value),
        cws_callback: fn(&Value),
    ) -> Result<Self> {
        Self::from_config(
            refresh_token,
            AlorConfig {
                demo,
                ws_callback,
                cws_callback,
                ..AlorConfig::default()
            },
        )
        .await
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

    pub async fn subscribe_orders_statuses_v2_and_wait_ack(
        &mut self,
        ws_rx: &mut broadcast::Receiver<Value>,
        exchange: &str,
        portfolio: &str,
        order_statuses: Option<Vec<String>>,
        skip_history: bool,
        frequency: i32,
        format: &str,
        timeout_duration: Duration,
    ) -> anyhow::Result<(Uuid, Value)> {
        let subscribe_guid = self
            .subscribe_orders_statuses_v2(
                exchange,
                portfolio,
                order_statuses,
                skip_history,
                frequency,
                format,
            )
            .await?;
        let subscribe_guid_s = subscribe_guid.to_string();

        let ack = match Self::wait_ws_event_by_guid(ws_rx, &subscribe_guid_s, Duration::from_secs(1)).await {
            Ok(evt) => evt,
            Err(_) => {
                let fut = async {
                    loop {
                        match ws_rx.recv().await {
                            Ok(event) => {
                                let top_guid_match =
                                    Self::ws_event_guid(&event).as_deref() == Some(&subscribe_guid_s);
                                let data_guid_match = event
                                    .get("data")
                                    .and_then(|d| d.get("guid"))
                                    .and_then(|v| v.as_str())
                                    .map(|g| g == subscribe_guid_s)
                                    .unwrap_or(false);
                                let request_guid_match = event
                                    .get("requestGuid")
                                    .and_then(|v| v.as_str())
                                    .map(|g| g == subscribe_guid_s)
                                    .unwrap_or(false);
                                let http_ok = event
                                    .get("httpCode")
                                    .and_then(|v| v.as_u64())
                                    .map(|code| code == 200)
                                    .unwrap_or(false);

                                if top_guid_match || data_guid_match || request_guid_match || http_ok {
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
                    .map_err(|_| anyhow!("Timeout waiting for OrdersGetAndSubscribeV2 ack"))??
            }
        };

        Ok((subscribe_guid, ack))
    }

    pub async fn subscribe_orders_statuses_v2_and_wait_ack_typed(
        &mut self,
        ws_rx: &mut broadcast::Receiver<Value>,
        exchange: &str,
        portfolio: &str,
        order_statuses: Option<Vec<String>>,
        skip_history: bool,
        frequency: i32,
        format: &str,
        timeout_duration: Duration,
    ) -> anyhow::Result<(Uuid, WsSubscribeAckEvent)> {
        let (guid, ack) = self
            .subscribe_orders_statuses_v2_and_wait_ack(
                ws_rx,
                exchange,
                portfolio,
                order_statuses,
                skip_history,
                frequency,
                format,
                timeout_duration,
            )
            .await?;
        Ok((guid, WsSubscribeAckEvent::from_raw(ack)))
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

    pub fn parse_cws_ack_event(event: Value) -> CwsAckEvent {
        CwsAckEvent::from_raw(event)
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

    pub fn parse_ws_order_status_event(event: Value) -> WsOrderStatusEvent {
        WsOrderStatusEvent::from_raw(event)
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

        let cws_ack_raw =
            Self::wait_cws_event_by_request_guid(cws_rx, &request_guid, timeout_duration).await?;
        let cws_ack = Self::parse_cws_ack_event(cws_ack_raw);

        if let Some(code) = cws_ack.http_code {
            if code != 200 {
                return Err(anyhow!("create_limit_order failed, cws ack: {}", cws_ack));
            }
        }

        let ws_status_event_raw = if let Some(order_id) = cws_ack.order_number.clone() {
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
        let ws_status_event = Self::parse_ws_order_status_event(ws_status_event_raw);

        let order_id = ws_status_event
            .order_id
            .clone()
            .or_else(|| cws_ack.order_number.clone())
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

        let cws_ack_raw =
            Self::wait_cws_event_by_request_guid(cws_rx, &request_guid, timeout_duration).await?;
        let cws_ack = Self::parse_cws_ack_event(cws_ack_raw);

        if let Some(code) = cws_ack.http_code {
            if code != 200 {
                return Err(anyhow!("delete_limit_order failed, cws ack: {}", cws_ack));
            }
        }

        let ws_status_event = Self::parse_ws_order_status_event(
            Self::wait_ws_order_status_by_id(ws_rx, order_id, timeout_duration).await?,
        );

        let resolved_order_id = ws_status_event
            .order_id
            .clone()
            .or_else(|| cws_ack.order_number.clone())
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

        let cws_ack_raw =
            Self::wait_cws_event_by_request_guid(cws_rx, &request_guid, timeout_duration).await?;
        let cws_ack = Self::parse_cws_ack_event(cws_ack_raw);

        if let Some(code) = cws_ack.http_code {
            if code != 200 {
                return Err(anyhow!("update_limit_order failed, cws ack: {}", cws_ack));
            }
        }

        let ack_order_id = cws_ack.order_number.clone();

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
                .ok()
                .map(Self::parse_ws_order_status_event);

                let new_order_status_event = Self::parse_ws_order_status_event(
                    Self::wait_ws_order_status_by_id(ws_rx, &new_order_id, timeout_duration)
                        .await?,
                );

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
        let new_order_status_event_raw = if let Some(same_id) = ack_order_id.clone() {
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
        let new_order_status_event = Self::parse_ws_order_status_event(new_order_status_event_raw);

        let new_order_id = new_order_status_event
            .order_id
            .clone()
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
    use super::{AlorClientBuilder, AlorConfig, AlorRust, CwsAckEvent, WsOrderStatusEvent, WsSubscribeAckEvent};
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
    fn cws_ack_event_from_raw_parses_fields() {
        let raw = json!({
            "httpCode": 200,
            "message": "ok",
            "requestGuid": "req-123",
            "orderNumber": "42"
        });
        let evt = CwsAckEvent::from_raw(raw.clone());
        assert_eq!(evt.http_code, Some(200));
        assert_eq!(evt.message.as_deref(), Some("ok"));
        assert_eq!(evt.request_guid.as_deref(), Some("req-123"));
        assert_eq!(evt.order_number.as_deref(), Some("42"));
        assert_eq!(evt.raw, raw);
    }

    #[test]
    fn ws_order_status_event_from_raw_parses_fields() {
        let raw = json!({
            "guid": "sub-1",
            "data": {
                "id": "2033125999100562015",
                "status": "working",
                "portfolio": "7502T0U",
                "symbol": "IMOEXF"
            }
        });
        let evt = WsOrderStatusEvent::from_raw(raw.clone());
        assert_eq!(evt.guid.as_deref(), Some("sub-1"));
        assert_eq!(evt.order_id.as_deref(), Some("2033125999100562015"));
        assert_eq!(evt.status.as_deref(), Some("working"));
        assert_eq!(evt.portfolio.as_deref(), Some("7502T0U"));
        assert_eq!(evt.symbol.as_deref(), Some("IMOEXF"));
        assert_eq!(evt.raw, raw);
    }

    #[test]
    fn ws_subscribe_ack_event_from_raw_parses_fields() {
        let raw = json!({
            "httpCode": 200,
            "message": "Handled successfully",
            "requestGuid": "sub-req-1"
        });
        let evt = WsSubscribeAckEvent::from_raw(raw.clone());
        assert_eq!(evt.http_code, Some(200));
        assert_eq!(evt.message.as_deref(), Some("Handled successfully"));
        assert_eq!(evt.request_guid.as_deref(), Some("sub-req-1"));
        assert_eq!(evt.raw, raw);
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

    #[test]
    fn alor_config_defaults_are_safe() {
        let cfg = AlorConfig::default();
        assert!(!cfg.demo);
        assert!(cfg.enable_ws);
        assert!(cfg.enable_cws);
        assert!(cfg.preload_jwt);
    }

    #[test]
    fn builder_mutates_config_flags() {
        let builder = AlorClientBuilder::new("token")
            .demo(true)
            .enable_ws(false)
            .enable_cws(true)
            .preload_jwt(false);
        assert_eq!(builder.config.demo, true);
        assert_eq!(builder.config.enable_ws, false);
        assert_eq!(builder.config.enable_cws, true);
        assert_eq!(builder.config.preload_jwt, false);
    }

    #[tokio::test]
    async fn from_config_rejects_disabled_ws_or_cws_before_network() {
        let result = AlorRust::from_config(
            "dummy-token",
            AlorConfig {
                enable_ws: false,
                ..AlorConfig::default()
            },
        )
        .await;
        let err = match result {
            Ok(_) => panic!("expected config validation error"),
            Err(err) => err,
        };
        assert!(err
            .to_string()
            .contains("enable_ws/enable_cws are not supported yet"));
    }
}
