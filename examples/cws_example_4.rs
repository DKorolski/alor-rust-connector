use alor_rust::dto::cws_dto::order_common::OrderSide;
use alor_rust::*;
use anyhow::Result;
use futures_util::sink::SinkExt;
use log::{debug, error, info};
use serde_json::Value;
use std::sync::OnceLock;
use std::time::Instant;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::time::{sleep, Duration};
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    // Инициализация логирования
    init_logger("INFO");

    // Каналы для получения событий из глобальных колбэков
    let (cws_event_tx, mut cws_event_rx) = unbounded_channel();
    let (ws_event_tx, ws_event_rx) = unbounded_channel();

    CWS_EVENT_SENDER
        .set(cws_event_tx)
        .map_err(|_| anyhow::anyhow!("Failed to set CWS event sender"))?;
    WS_EVENT_SENDER
        .set(ws_event_tx)
        .map_err(|_| anyhow::anyhow!("Failed to set WS event sender"))?;

    // Создание клиента для взаимодействия с API (WS + CWS)
    let mut client = AlorRust::new(
        "f4284595-00ba-43c9-a304-68f53b6b4b6a",          // Твой refresh token
        false,       // demo=false для боевого API
        log_ws_event, // Колбэк для WS (подписка на статусы заявок)
        log_cws_event, // Колбэк для CWS (создание/изменение заявок)
    )
    .await?;

    let portfolio = "7502T0U"; // Убедись, что это правильный портфель

    // Запускаем отдельную задачу, чтобы сразу логировать приходящие WS-события по заявкам
    tokio::spawn(async move {
        log_ws_status_stream(ws_event_rx).await;
    });

    // Подписка на статусы заявок (WS)
    let subscribe_guid = subscribe_order_statuses(&mut client, portfolio).await?;
    info!(
        "Subscribed to order status stream via WS, guid: {}",
        subscribe_guid
    );

    // Получаем позиции для портфеля
    let positions = client
        .get_positions(portfolio, "MOEX", true, "Simple")
        .await?;

    // Проверка наличия позиций
    if let Some(positions_array) = positions.as_array() {
        if positions_array.is_empty() {
            info!("No positions found for portfolio {}", portfolio);
        } else {
            println!("portfolio: {:?}\tpositions: {:?}", portfolio, positions_array);
        }
    } else {
        error!("Failed to parse positions for portfolio {}", portfolio);
        return Ok(());
    }

    // Создание лимитного ордера
    info!("Creating limit order...");
    let create_started = Instant::now();
    let create_guid = client
        .cws_client
        .create_limit_order(
            OrderSide::Buy,
            1,
            2700.0,
            "IMOEXF",
            "MOEX",
            None,
            portfolio,
            None,
            None,
            Some(true),
            None,
            None,
            None,
        )
        .await?;
    info!("create_limit_order request guid: {:?}", create_guid);

    // Ждём первое событие с номером заявки
    let mut order_number = match wait_for_order_number(&mut cws_event_rx, Some(create_guid.as_str())).await {
        Some(number) => number,
        None => {
            error!("No order number received for created order");
            return Ok(());
        }
    };

    info!("Created order number: {}", order_number);
    info!(
        "Timing: create flow took {} ms",
        create_started.elapsed().as_millis()
    );

    // Дадим время на приход статуса заявки (например, Working) по WS до обновления/удаления.
    sleep(Duration::from_millis(300)).await;

    // Обновление лимитного ордера с новой ценой
    info!("Updating limit order with new price: 2710.0");
    let update_started = Instant::now();
    let update_guid = client
        .cws_client
        .update_limit_order(
            &order_number,
            OrderSide::Buy,
            1,
            2710.0,
            "IMOEXF",
            "MOEX",
            None,
            portfolio,
            None,
            Some(true),
            None,
            None,
        )
        .await?;
    info!("update_limit_order request guid: {:?}", update_guid);

    // Если сервер вернёт новый orderNumber после апдейта, подменяем его
    if let Some(updated_number) =
        wait_for_order_number(&mut cws_event_rx, Some(update_guid.as_str())).await
    {
        order_number = updated_number;
        info!("Order number after update: {}", order_number);
    }
    info!(
        "Timing: update flow took {} ms",
        update_started.elapsed().as_millis()
    );

    // Удаление лимитного ордера с актуальным orderNumber
    info!("Deleting limit order with orderNumber: {}", order_number);
    let delete_started = Instant::now();
    let delete_guid = client
        .cws_client
        .delete_limit_order(&order_number, "MOEX", portfolio, None)
        .await?;
    info!("delete_limit_order request guid: {:?}", delete_guid);

    // Ждём подтверждение удаления по requestGuid (опционально)
    if let Some(delete_event) = wait_for_event_by_request_guid(&mut cws_event_rx, delete_guid.as_str()).await {
        info!("Delete response: {:?}", delete_event);
    }
    info!(
        "Timing: delete flow took {} ms",
        delete_started.elapsed().as_millis()
    );

    // Немного ждём перед завершением, чтобы обработать хвост событий
    sleep(Duration::from_secs(3)).await;

    Ok(())
}

// Подписка на статусы заявок через WS
async fn subscribe_order_statuses(client: &mut AlorRust, portfolio: &str) -> Result<Uuid> {
    let token_started = Instant::now();
    let token_json: Value = serde_json::from_str(
        client
            .auth_client
            .get_jwt_token()
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
            .as_str(),
    )?;
    info!(
        "Timing: get_access_token took {} ms",
        token_started.elapsed().as_millis()
    );
    let access_token = token_json
        .get("AccessToken")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("AccessToken not found in JWT response"))?;

    let mut request_body = Value::Object(serde_json::Map::new());
    let subscribe_guid = Uuid::new_v4();

    if let Some(body) = request_body.as_object_mut() {
        body.insert(
            "opcode".to_string(),
            Value::from("OrdersGetAndSubscribeV2"),
        );
        body.insert("exchange".to_string(), Value::from("MOEX"));
        body.insert("portfolio".to_string(), Value::from(portfolio));
        body.insert("skipHistory".to_string(), Value::from(true));
        body.insert("frequency".to_string(), Value::from(0));
        body.insert("format".to_string(), Value::from("Simple"));
        body.insert("token".to_string(), Value::from(access_token));
        body.insert("guid".to_string(), Value::from(subscribe_guid.to_string()));
    }

    let message = Message::Text(Utf8Bytes::from(request_body.to_string()));

    let subscribe_started = Instant::now();
    client
        .client
        .socket_client
        .write_stream
        .send(message)
        .await?;
    info!(
        "Timing: subscribe order status request took {} ms",
        subscribe_started.elapsed().as_millis()
    );

    Ok(subscribe_guid)
}

// Функция для логирования событий CWS
fn log_cws_event(event: &Value) {
    info!("CWS event: {:?}", event);

    if let Some(sender) = CWS_EVENT_SENDER.get() {
        let _ = sender.send(event.clone());
    }
}

// Функция для логирования событий WS
fn log_ws_event(event: &Value) {
    info!("WS event: {:?}", event);

    if let Some(sender) = WS_EVENT_SENDER.get() {
        let _ = sender.send(event.clone());
    }
}

// Фоновый обработчик для печати статусов заявок, приходящих через WS
async fn log_ws_status_stream(mut event_rx: UnboundedReceiver<Value>) {
    while let Some(event) = event_rx.recv().await {
        let entries = extract_status_entries(&event);

        if entries.is_empty() {
            info!("WS status update: <unparsed>, raw={:?}", event);
        } else {
            for (order, status) in entries {
                info!("WS status update: order={}, status={}", order, status);
            }
        }
    }
}

/// Извлекает статусы заявок из разных форматов WS-событий
fn extract_status_entries(event: &Value) -> Vec<(String, String)> {
    let mut entries = Vec::new();

    // Некоторые ответы приходят как массив, некоторые — как объект, а иногда
    // вложены в поле orders/Orders. Бывают и плоские ответы без data, поэтому
    // обрабатываем событие целиком и вложенные структуры.
    let mut items: Vec<&Value> = Vec::new();

    // 1) Сначала пытаемся брать записи из event["data"], если оно есть.
    if let Some(data) = event.get("data") {
        if let Some(array) = data.as_array() {
            items.extend(array.iter());
        } else if let Some(obj) = data.as_object() {
            if let Some(orders) = obj
                .get("orders")
                .or_else(|| obj.get("Orders"))
                .and_then(|v| v.as_array())
            {
                items.extend(orders.iter());
            } else {
                items.push(data);
            }
        } else {
            items.push(data);
        }
    }

    // 2) Если event сам по себе массив — рассматриваем его элементы как записи.
    if items.is_empty() {
        if let Some(array) = event.as_array() {
            items.extend(array.iter());
        }
    }

    // 3) В крайнем случае используем всё событие целиком как единственную запись.
    if items.is_empty() {
        items.push(event);
    }

    for item in items {
        let order_number = item
            .get("orderno")
            .or_else(|| item.get("orderNo"))
            .or_else(|| item.get("orderNumber"))
            .or_else(|| item.get("orderId"))
            .or_else(|| item.get("id"))
            .and_then(|v| {
                if v.is_string() {
                    v.as_str().map(|s| s.to_string())
                } else if v.is_number() {
                    v.as_i64().map(|n| n.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "<no order number>".to_string());

        let status = item
            .get("status")
            .and_then(|v| v.as_str())
            .or_else(|| {
                item.get("statusExtended")
                    .and_then(|ext| ext.get("status"))
                    .and_then(|v| v.as_str())
            })
            .or_else(|| {
                item.get("statusExtended")
                    .and_then(|ext| ext.get("state"))
                    .and_then(|v| v.as_str())
            })
            .or_else(|| {
                item.get("statusExtended")
                    .and_then(|ext| ext.get("statusName"))
                    .and_then(|v| v.as_str())
            })
            .map(|s| s.to_string())
            .unwrap_or_else(|| "<unknown>".to_string());

        entries.push((order_number, status));
    }

    entries
}

// Извлекает orderNumber из события
fn extract_order_number(event: &Value) -> Option<String> {
    event
        .get("orderNumber")
        .or_else(|| event.get("orderno"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Ждёт событие с нужным requestGuid и наличием orderNumber
async fn wait_for_order_number(
    event_rx: &mut UnboundedReceiver<Value>,
    request_guid: Option<&str>,
) -> Option<String> {
    while let Some(event) = event_rx.recv().await {
        if let Some(guid) = request_guid {
            let matches_guid = event
                .get("requestGuid")
                .and_then(|v| v.as_str())
                .map(|request| request == guid)
                .unwrap_or(false);

            if !matches_guid {
                debug!("Skip event with another requestGuid (wanted {}): {:?}", guid, event);
                continue;
            }
        }

        if let Some(order_number) = extract_order_number(&event) {
            debug!("Received order event: {:?}", event);
            return Some(order_number);
        }

        debug!("Skip non-order event: {:?}", event);
    }

    None
}

/// Ждёт событие по requestGuid (не обязательно содержащее orderNumber)
async fn wait_for_event_by_request_guid(
    event_rx: &mut UnboundedReceiver<Value>,
    request_guid: &str,
) -> Option<Value> {
    while let Some(event) = event_rx.recv().await {
        if event
            .get("requestGuid")
            .and_then(|v| v.as_str())
            .map(|guid| guid == request_guid)
            .unwrap_or(false)
        {
            debug!("Received event for requestGuid {}: {:?}", request_guid, event);
            return Some(event);
        }

        debug!(
            "Skip event with another requestGuid (wanted {}): {:?}",
            request_guid, event
        );
    }

    None
}

static CWS_EVENT_SENDER: OnceLock<UnboundedSender<Value>> = OnceLock::new();
static WS_EVENT_SENDER: OnceLock<UnboundedSender<Value>> = OnceLock::new();