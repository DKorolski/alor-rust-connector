use futures_util::stream::{SplitSink, SplitStream};
use futures_util::StreamExt;
use log::{debug, error, info, warn};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::watch::{channel, Receiver};
use tokio_tungstenite::tungstenite::http::Uri;
use tokio_tungstenite::{
    connect_async, tungstenite::protocol::Message, MaybeTlsStream, WebSocketStream,
};

#[derive(Debug)]
pub struct WebSocketState {
    pub write_stream: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    events_tx: broadcast::Sender<Value>,
}

impl WebSocketState {
    async fn socket_listener(
        mut stream: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
        _api_response_tx: mpsc::Sender<Value>,
        events_tx: broadcast::Sender<Value>,
        mut shutdown_rx: Receiver<bool>,
        callback: fn(&Value),
    ) {
        debug!("Starting WebSocketListener");

        let mut prev_bar: Option<Value> = None;
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
                                            let _ = events_tx.send(api_response.clone());
                                            if api_response.get("data").is_none() {
                                                callback(&api_response);
                                                continue;
                                            }

                                            let current_bar_seconds = api_response
                                                .get("data")
                                                .and_then(|d| d.get("time"))
                                                .and_then(|t| t.as_u64());

                                            // Non-bar WS events (e.g. order statuses) do not have data.time.
                                            // Forward them to the user callback without bar aggregation logic.
                                            if current_bar_seconds.is_none() {
                                                callback(&api_response);
                                                continue;
                                            }

                                            if prev_bar.is_none() {
                                                prev_bar = Some(api_response);
                                                continue;
                                            } else {
                                                let Some(prev_bar_data) = prev_bar.clone() else {
                                                    prev_bar = Some(api_response);
                                                    continue;
                                                };

                                                let Some(current_bar_secconds) = current_bar_seconds else {
                                                    callback(&api_response);
                                                    continue;
                                                };
                                                let prev_bar_secconds = prev_bar_data
                                                    .get("data")
                                                    .and_then(|d| d.get("time"))
                                                    .and_then(|t| t.as_u64());

                                                let Some(prev_bar_secconds) = prev_bar_secconds else {
                                                    // Previous event was not a regular bar, reset aggregation state.
                                                    prev_bar = Some(api_response);
                                                    continue;
                                                };

                                                if current_bar_secconds == prev_bar_secconds {  // обновленная версия текущего бара
                                                    prev_bar = Some(api_response);
                                                } else if current_bar_secconds > prev_bar_secconds {
                                                    debug!("websocket_handler: OnNewBar {:?}", prev_bar_data);
                                                    // raise callback on previous bar
                                                    callback(&prev_bar_data);
                                                    // set current bar as previous bar
                                                    prev_bar = Some(api_response);
                                                }
                                            }

                                        }
                                        Err(e) => {
                                            warn!("Не JSON: {}. Данные: {}", e, txt);
                                        }
                                    }
                                }
                                Message::Binary(bin) => { // Обработка бинарных данных
                                     debug!("Бинарные данные: {} байт", bin.len());
                                }
                                Message::Ping(_) => {
                                    debug!("Получен Ping")
                                },
                                Message::Pong(_) => {},
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

    pub async fn new(url: Uri, callback: fn(&Value)) -> anyhow::Result<Self> {
        info!("Подключение к Alor WebSocket...");

        let (ws_stream, response) = connect_async(url).await?;
        debug!("Connect response: {}", response.status());

        let (write, read) = ws_stream.split();

        // Канал для сообщений клиента -> Writer Task
        // let (client_message_tx, client_message_rx) = mpsc::channel::<Value>(32);
        // Канал для сообщений API -> Внешний мир (пользователь клиента)
        let (api_response_tx, _api_response_rx) = mpsc::channel::<Value>(100); // Буфер для полученных сообщений
                                                                               // Канал для сигнала завершения
        let (events_tx, _) = broadcast::channel::<Value>(1024);
        let (_shutdown_tx, shutdown_rx) = channel(false);

        // Запускаем задачи чтения и записи
        let reader_shutdown_rx = shutdown_rx.clone();

        tokio::spawn(Self::socket_listener(
            read,
            api_response_tx,
            events_tx.clone(),
            reader_shutdown_rx,
            callback,
        ));

        info!("Успешно подключено!");
        Ok(WebSocketState {
            write_stream: write,
            events_tx,
            // listener_inited: false,
        })
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<Value> {
        self.events_tx.subscribe()
    }
}
