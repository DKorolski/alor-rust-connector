use chrono::Utc;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info, warn};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::watch::{channel, Receiver};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::tungstenite::http::Uri;
use tokio_tungstenite::{
    connect_async, tungstenite::protocol::Message, MaybeTlsStream, WebSocketStream,
};

use anyhow::{anyhow, Result};

use crate::dto::cws_dto::create_limit_order_dto::CreateLimitOrderRequest;
use crate::dto::cws_dto::create_market_order_dto::CreateMarketOrderRequest;
use crate::dto::cws_dto::create_stop_limit_order_dto::CreateStopLimitOrderRequest;
use crate::dto::cws_dto::create_stop_order_dto::CreateStopOrderRequest;
use crate::dto::cws_dto::delete_order_dto::DeleteOrderRequest;
use crate::dto::cws_dto::order_common::{Instrument, OrderSide, StopCondition, TimeInForce, User};
use crate::dto::cws_dto::update_limit_order_dto::UpdateLimitOrderRequest;
use crate::dto::cws_dto::update_market_order_dto::UpdateMarketOrderRequest;
use crate::dto::cws_dto::update_stop_limit_order_dto::UpdateStopLimitOrderRequest;
use crate::dto::cws_dto::update_stop_order_dto::UpdateStopOrderRequest;
use crate::helpers::servers::get_server_url;
use crate::structs::auth_client::AuthClient;

// Тип для callback-функций обработчиков ответов
type ResponseCallback = Box<dyn Fn(Value) + Send + Sync>;

pub struct CWS {
    pub write_stream: Arc<Mutex<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>,
    pub request_callbacks: Arc<Mutex<HashMap<String, ResponseCallback>>>,
    auth_client: AuthClient,
    last_auth: u64,
}

impl CWS {
    pub async fn new(refresh_token: &str, demo: bool, client_callback: fn(&Value)) -> Result<Self> {
        info!("Подключение к Alor CWS...");
        let url = Uri::from_str(get_server_url("cws_server", demo)?.as_str())?;

        let (ws_stream, response) = connect_async(url).await?;
        debug!("Connect response: {}", response.status());

        let (write, read) = ws_stream.split();
        let write_stream = Arc::new(Mutex::new(write));

        // Хранилище для callback-функций по GUID запросов
        let request_callbacks: Arc<Mutex<HashMap<String, ResponseCallback>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Канал для сигнала завершения
        let (_shutdown_tx, shutdown_rx) = channel(false);

        // Запускаем задачу чтения
        let callbacks_clone = request_callbacks.clone();
        tokio::spawn(Self::socket_listener(
            read,
            callbacks_clone,
            shutdown_rx,
            client_callback,
        ));

        info!("Успешно подключено!");
        Ok(CWS {
            write_stream,
            auth_client: AuthClient::new(refresh_token, demo)?,
            request_callbacks,
            last_auth: 0,
        })
    }

    async fn socket_listener(
        mut stream: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
        request_callbacks: Arc<Mutex<HashMap<String, ResponseCallback>>>,
        mut shutdown_rx: Receiver<bool>,
        global_callback: fn(&Value),
    ) {
        debug!("Starting WebSocketListener");

        loop {
            tokio::select! {
                // Ожидаем сообщение от WebSocket ИЛИ сигнал завершения
                msg_result = stream.next() => {
                    match msg_result {
                        Some(Ok(msg)) => {
                            match msg {
                                Message::Text(txt) => {
                                    match serde_json::from_str::<Value>(&txt) {
                                        Ok(api_response) => {
                                            debug!("Получен JSON: {:?}", api_response);

                                            // Проверяем, есть ли requestGuid в ответе для сопоставления с запросом
                                            let request_guid = api_response.get("requestGuid").and_then(|v| v.as_str()).map(|s| s.to_string());

                                            if let Some(request_guid) = request_guid {
                                                let callbacks = request_callbacks.clone();
                                                let response = api_response.clone();

                                                tokio::spawn(async move {
                                                    let mut callbacks_lock = callbacks.lock().await;
                                                    if let Some(callback) = callbacks_lock.remove(&request_guid) {
                                                        callback(response);
                                                    } else {
                                                        global_callback(&response);
                                                    }
                                                });
                                            } else {
                                                // Обработка сообщений без requestGuid (например, пуш-уведомлений)
                                                debug!("Получено сообщение без requestGuid: {:?}", api_response);
                                                global_callback(&api_response);
                                            }
                                        }
                                        Err(e) => {
                                            warn!("Не JSON: {}. Данные: {}", e, txt);
                                        }
                                    }
                                }
                                Message::Binary(bin) => {
                                    debug!("Бинарные данные: {} байт", bin.len());
                                }
                                Message::Ping(_) => {
                                    debug!("Получен Ping")
                                },
                                Message::Pong(_) => {
                                    // debug!("Получен Pong");
                                },
                                Message::Close(frame) => {
                                    debug!("Соединение закрыто сервером: {:?}", frame);
                                    break;
                                }
                                Message::Frame(_) => warn!("Неожиданный Frame"),
                            }
                        },
                        Some(Err(e)) => {
                            error!("Ошибка чтения: {}", e);
                            break;
                        },
                        None => {
                            info!("Поток чтения завершен.");
                            break;
                        }
                    }
                }
                // Проверяем сигнал завершения
                _ = shutdown_rx.changed() => {
                     if *shutdown_rx.borrow() {
                        info!("Получен сигнал завершения.");
                        break;
                     }
                }
            }
        }
        info!("Задача чтения завершена.");
    }

    /// Создание рыночной заявки
    pub async fn create_market_order(
        &mut self,
        side: OrderSide,
        quantity: i32,
        symbol: &str,
        exchange: &str,
        instrument_group: Option<&str>,
        portfolio: &str,
        comment: Option<String>,
        time_in_force: Option<TimeInForce>,
        allow_margin: Option<bool>,
        check_duplicates: Option<bool>,
    ) -> Result<String> {
        let guid = uuid::Uuid::new_v4().to_string();

        let request = CreateMarketOrderRequest {
            opcode: "create:market".to_string(),
            guid: guid.clone(),
            side,
            quantity,
            instrument: Instrument {
                symbol: symbol.to_string(),
                exchange: exchange.to_string(),
                instrument_group: instrument_group.map(|s| s.to_string()),
            },
            comment,
            user: User {
                portfolio: portfolio.to_string(),
            },
            time_in_force,
            allow_margin,
            check_duplicates,
        };

        // Отправляем сообщение
        let text_message = serde_json::to_string(&request)?;
        self.send_message(text_message).await?;

        // Возвращаем GUID для отслеживания ответа
        Ok(guid)
    }

    /// Создание лимитной заявки
    pub async fn create_limit_order(
        &mut self,
        side: OrderSide,
        quantity: i32,
        price: f64,
        symbol: &str,
        exchange: &str,
        instrument_group: Option<&str>,
        portfolio: &str,
        comment: Option<String>,
        time_in_force: Option<TimeInForce>,
        allow_margin: Option<bool>,
        iceberg_fixed: Option<i32>,
        iceberg_variance: Option<i32>,
        check_duplicates: Option<bool>,
    ) -> Result<String> {
        let guid = uuid::Uuid::new_v4().to_string();

        let request = CreateLimitOrderRequest {
            opcode: "create:limit".to_string(),
            guid: guid.clone(),
            side,
            quantity,
            price,
            instrument: Instrument {
                symbol: symbol.to_string(),
                exchange: exchange.to_string(),
                instrument_group: instrument_group.map(|s| s.to_string()),
            },
            comment,
            user: User {
                portfolio: portfolio.to_string(),
            },
            time_in_force,
            allow_margin,
            iceberg_fixed,
            iceberg_variance,
            check_duplicates,
        };

        // Отправляем сообщение
        let text_message = serde_json::to_string(&request)?;
        self.send_message(text_message).await?;

        // Возвращаем GUID для отслеживания ответа
        Ok(guid)
    }

    /// Создание стоп-заявки
    pub async fn create_stop_order(
        &mut self,
        side: OrderSide,
        quantity: i32,
        condition: StopCondition,
        trigger_price: f64,
        symbol: &str,
        exchange: &str,
        instrument_group: Option<&str>,
        portfolio: &str,
        stop_end_unix_time: Option<i64>,
        check_duplicates: Option<bool>,
        allow_margin: Option<bool>,
        protecting_seconds: Option<i32>,
        comment: Option<String>,
        activate: Option<bool>,
    ) -> Result<String> {
        let guid = uuid::Uuid::new_v4().to_string();

        let request = CreateStopOrderRequest {
            opcode: "create:stop".to_string(),
            guid: guid.clone(),
            side,
            quantity,
            condition,
            trigger_price,
            stop_end_unix_time,
            instrument: Instrument {
                symbol: symbol.to_string(),
                exchange: exchange.to_string(),
                instrument_group: instrument_group.map(|s| s.to_string()),
            },
            user: User {
                portfolio: portfolio.to_string(),
            },
            check_duplicates,
            allow_margin,
            protecting_seconds,
            comment,
            activate,
        };

        // Отправляем сообщение
        let text_message = serde_json::to_string(&request)?;
        self.send_message(text_message).await?;

        // Возвращаем GUID для отслеживания ответа
        Ok(guid)
    }

    /// Создание стоп-лимитной заявки
    pub async fn create_stop_limit_order(
        &mut self,
        side: OrderSide,
        quantity: i32,
        condition: StopCondition,
        price: f64,
        trigger_price: f64,
        symbol: &str,
        exchange: &str,
        instrument_group: Option<&str>,
        portfolio: &str,
        time_in_force: Option<TimeInForce>,
        stop_end_unix_time: Option<i64>,
        allow_margin: Option<bool>,
        iceberg_fixed: Option<i32>,
        iceberg_variance: Option<i32>,
        check_duplicates: Option<bool>,
        protecting_seconds: Option<i32>,
        comment: Option<String>,
        activate: Option<bool>,
    ) -> Result<String> {
        let guid = uuid::Uuid::new_v4().to_string();

        let request = CreateStopLimitOrderRequest {
            opcode: "create:stopLimit".to_string(),
            guid: guid.clone(),
            side,
            quantity,
            price,
            condition,
            trigger_price,
            stop_end_unix_time,
            instrument: Instrument {
                symbol: symbol.to_string(),
                exchange: exchange.to_string(),
                instrument_group: instrument_group.map(|s| s.to_string()),
            },
            user: User {
                portfolio: portfolio.to_string(),
            },
            time_in_force,
            allow_margin,
            iceberg_fixed,
            iceberg_variance,
            check_duplicates,
            protecting_seconds,
            comment,
            activate,
        };

        // Отправляем сообщение
        let text_message = serde_json::to_string(&request)?;
        self.send_message(text_message).await?;

        // Возвращаем GUID для отслеживания ответа
        Ok(guid)
    }

    /// Изменение рыночной заявки
    pub async fn update_market_order(
        &mut self,
        order_id: &str,
        side: OrderSide,
        quantity: i32,
        symbol: &str,
        exchange: &str,
        instrument_group: Option<&str>,
        portfolio: &str,
        comment: Option<String>,
        time_in_force: Option<TimeInForce>,
        allow_margin: Option<bool>,
        check_duplicates: Option<bool>,
    ) -> Result<String> {
        let guid = uuid::Uuid::new_v4().to_string();

        let request = UpdateMarketOrderRequest {
            opcode: "update:market".to_string(),
            guid: guid.clone(),
            order_id: order_id.to_string(),
            side,
            quantity,
            instrument: Instrument {
                symbol: symbol.to_string(),
                exchange: exchange.to_string(),
                instrument_group: instrument_group.map(|s| s.to_string()),
            },
            comment,
            user: User {
                portfolio: portfolio.to_string(),
            },
            time_in_force,
            allow_margin,
            check_duplicates,
        };

        // Отправляем сообщение
        let text_message = serde_json::to_string(&request)?;
        self.send_message(text_message).await?;

        // Возвращаем GUID для отслеживания ответа
        Ok(guid)
    }

    /// Изменение лимитной заявки
    pub async fn update_limit_order(
        &mut self,
        order_id: &str,
        side: OrderSide,
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
    ) -> Result<String> {
        let guid = uuid::Uuid::new_v4().to_string();

        let request = UpdateLimitOrderRequest {
            opcode: "update:limit".to_string(),
            guid: guid.clone(),
            order_id: order_id.to_string(),
            side,
            quantity,
            price,
            instrument: Instrument {
                symbol: symbol.to_string(),
                exchange: exchange.to_string(),
                instrument_group: instrument_group.map(|s| s.to_string()),
            },
            comment,
            user: User {
                portfolio: portfolio.to_string(),
            },
            allow_margin,
            iceberg_fixed,
            check_duplicates,
        };

        // Отправляем сообщение
        let text_message = serde_json::to_string(&request)?;
        self.send_message(text_message).await?;

        // Возвращаем GUID для отслеживания ответа
        Ok(guid)
    }

    /// Изменение стоп-заявки
    pub async fn update_stop_order(
        &mut self,
        order_id: &str,
        side: OrderSide,
        quantity: i32,
        condition: StopCondition,
        trigger_price: f64,
        symbol: &str,
        exchange: &str,
        instrument_group: Option<&str>,
        portfolio: &str,
        stop_end_unix_time: Option<i64>,
        allow_margin: Option<bool>,
        check_duplicates: Option<bool>,
        protecting_seconds: Option<i32>,
        comment: Option<String>,
        activate: Option<bool>,
    ) -> Result<String> {
        let guid = uuid::Uuid::new_v4().to_string();

        let request = UpdateStopOrderRequest {
            opcode: "update:stop".to_string(),
            guid: guid.clone(),
            order_id: order_id.to_string(),
            side,
            quantity,
            condition,
            trigger_price,
            stop_end_unix_time,
            instrument: Instrument {
                symbol: symbol.to_string(),
                exchange: exchange.to_string(),
                instrument_group: instrument_group.map(|s| s.to_string()),
            },
            user: User {
                portfolio: portfolio.to_string(),
            },
            allow_margin,
            check_duplicates,
            protecting_seconds,
            comment,
            activate,
        };

        // Отправляем сообщение
        let text_message = serde_json::to_string(&request)?;
        self.send_message(text_message).await?;

        // Возвращаем GUID для отслеживания ответа
        Ok(guid)
    }

    /// Изменение стоп-лимитной заявки
    pub async fn update_stop_limit_order(
        &mut self,
        order_id: &str,
        side: OrderSide,
        quantity: i32,
        condition: StopCondition,
        price: f64,
        trigger_price: f64,
        symbol: &str,
        exchange: &str,
        instrument_group: Option<&str>,
        portfolio: &str,
        time_in_force: Option<TimeInForce>,
        stop_end_unix_time: Option<i64>,
        allow_margin: Option<bool>,
        iceberg_fixed: Option<i32>,
        check_duplicates: Option<bool>,
        protecting_seconds: Option<i32>,
        comment: Option<String>,
        activate: Option<bool>,
    ) -> Result<String> {
        let guid = uuid::Uuid::new_v4().to_string();

        let request = UpdateStopLimitOrderRequest {
            opcode: "update:stopLimit".to_string(),
            guid: guid.clone(),
            order_id: order_id.to_string(),
            side,
            quantity,
            price,
            condition,
            trigger_price,
            stop_end_unix_time,
            instrument: Instrument {
                symbol: symbol.to_string(),
                exchange: exchange.to_string(),
                instrument_group: instrument_group.map(|s| s.to_string()),
            },
            comment,
            user: User {
                portfolio: portfolio.to_string(),
            },
            time_in_force,
            allow_margin,
            iceberg_fixed,
            check_duplicates,
            protecting_seconds,
            activate,
        };

        // Отправляем сообщение
        let text_message = serde_json::to_string(&request)?;
        self.send_message(text_message).await?;

        // Возвращаем GUID для отслеживания ответа
        Ok(guid)
    }

    /// Снятие рыночной заявки
    pub async fn delete_market_order(
        &mut self,
        order_id: &str,
        exchange: &str,
        portfolio: &str,
        check_duplicates: Option<bool>,
    ) -> Result<String> {
        let guid = uuid::Uuid::new_v4().to_string();

        let request = DeleteOrderRequest {
            opcode: "delete:market".to_string(),
            guid: guid.clone(),
            order_id: order_id.to_string(),
            exchange: exchange.to_string(),
            user: User {
                portfolio: portfolio.to_string(),
            },
            check_duplicates,
        };

        // Отправляем сообщение
        let text_message = serde_json::to_string(&request)?;
        self.send_message(text_message).await?;

        // Возвращаем GUID для отслеживания ответа
        Ok(guid)
    }
    
    /// Снятие лимитной заявки
    pub async fn delete_limit_order(
        &mut self,
        order_id: &str,
        exchange: &str,
        portfolio: &str,
        check_duplicates: Option<bool>,
    ) -> Result<String> {
        let guid = uuid::Uuid::new_v4().to_string();

        let request = DeleteOrderRequest {
            opcode: "delete:limit".to_string(),
            guid: guid.clone(),
            order_id: order_id.to_string(),
            exchange: exchange.to_string(),
            user: User {
                portfolio: portfolio.to_string(),
            },
            check_duplicates,
        };

        // Отправляем сообщение
        let text_message = serde_json::to_string(&request)?;
        self.send_message(text_message).await?;

        // Возвращаем GUID для отслеживания ответа
        Ok(guid)
    }
    
    /// Снятие стоп-заявки
    pub async fn delete_stop_order(
        &mut self,
        order_id: &str,
        exchange: &str,
        portfolio: &str,
        check_duplicates: Option<bool>,
    ) -> Result<String> {
        let guid = uuid::Uuid::new_v4().to_string();

        let request = DeleteOrderRequest {
            opcode: "delete:stop".to_string(),
            guid: guid.clone(),
            order_id: order_id.to_string(),
            exchange: exchange.to_string(),
            user: User {
                portfolio: portfolio.to_string(),
            },
            check_duplicates,
        };

        // Отправляем сообщение
        let text_message = serde_json::to_string(&request)?;
        self.send_message(text_message).await?;

        // Возвращаем GUID для отслеживания ответа
        Ok(guid)
    }
    
    /// Снятие стоп-лимитной заявки
    pub async fn delete_stop_limit_order(
        &mut self,
        order_id: &str,
        exchange: &str,
        portfolio: &str,
        check_duplicates: Option<bool>,
    ) -> Result<String> {
        let guid = uuid::Uuid::new_v4().to_string();

        let request = DeleteOrderRequest {
            opcode: "delete:stopLimit".to_string(),
            guid: guid.clone(),
            order_id: order_id.to_string(),
            exchange: exchange.to_string(),
            user: User {
                portfolio: portfolio.to_string(),
            },
            check_duplicates,
        };

        // Отправляем сообщение
        let text_message = serde_json::to_string(&request)?;
        self.send_message(text_message).await?;

        // Возвращаем GUID для отслеживания ответа
        Ok(guid)
    }

    // Метод для отправки сообщения и регистрации callback-а для обработки ответа
    async fn send_message(&mut self, message: String) -> Result<()> {
        if self.auth_required() {
            debug!("update auth for cws socket");
            self.authenticate().await?;
        }

        let mut write_stream = self.write_stream.lock().await;

        debug!("Отправка сообщения: {}", message);
        if let Err(e) = write_stream.send(Message::text(message)).await {
            error!("Ошибка отправки сообщения: {}", e);
            return Err(e.into());
        }

        Ok(())
    }

    // Примеры методов для отправки различных типов сообщений
    async fn authenticate(&mut self) -> Result<Value> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);

        // Генерируем уникальный GUID для запроса
        let guid = uuid::Uuid::new_v4().to_string();
        let auth_token_raw: Value =
            serde_json::from_str(self.auth_client.get_jwt_token_force().await?.as_str())?;
        let auth_token = auth_token_raw
            .get("AccessToken")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("AccessToken missing in JWT response"))?
            .to_string();

        let message = json!({
            "opcode": "authorize",
            "guid": guid,
            "token": auth_token
        });

        // Регистрируем callback для этого GUID
        {
            let tx_clone = tx.clone();
            let mut callbacks = self.request_callbacks.lock().await;
            callbacks.insert(
                guid.clone(),
                Box::new(move |response| {
                    let tx_clone = tx_clone.clone();
                    tokio::spawn(async move {
                        let _ = tx_clone.send(response).await;
                    });
                }),
            );
        }

        // Отправляем сообщение
        let text_message = serde_json::to_string(&message)?;
        debug!("Отправка сообщения: {}", text_message);

        let mut write_stream = self.write_stream.lock().await;
        if let Err(e) = write_stream.send(Message::text(text_message)).await {
            error!("Ошибка отправки сообщения: {}", e);
            // Удаляем callback в случае ошибки отправки
            let mut callbacks = self.request_callbacks.lock().await;
            callbacks.remove(&guid);
            return Err(e.into());
        }

        // Ждем ответ
        let recv_result = timeout(Duration::from_secs(10), rx.recv()).await;
        let result = if let Ok(Some(response)) = recv_result {
            // Проверяем код ответа
            if let Some(http_code) = response.get("httpCode").and_then(|v| v.as_u64()) {
                if http_code == 200 {
                    Ok(response)
                } else {
                    let error_message = response
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown error");
                    Err(anyhow!(
                        "Authentication failed with HTTP code {}: {}",
                        http_code,
                        error_message
                    ))
                }
            } else {
                Err(anyhow!("Invalid authorization response format"))
            }
        } else if let Ok(None) = recv_result {
            Err(anyhow!("Authorization response channel closed"))
        } else {
            // timeout elapsed
            let mut callbacks = self.request_callbacks.lock().await;
            callbacks.remove(&guid);
            Err(anyhow!("No authorization response received within timeout"))
        };
        if result.is_ok() {
            self.last_auth = Utc::now().timestamp() as u64;
        }
        result
    }

    fn auth_required(&self) -> bool {
        debug!(
            "last_auth: {:?}, now: {:?}, auth_required: {:?}",
            self.last_auth,
            Utc::now().timestamp(),
            (self.last_auth <= (Utc::now().timestamp() as u64 - 20 * 60))
        );
        self.last_auth <= (Utc::now().timestamp() as u64 - 20 * 60)
    }
}
