use alor_rust::dto::cws_dto::order_common::OrderSide;
use alor_rust::dto::cws_dto::order_common::TimeInForce;
use alor_rust::*;
use anyhow::{anyhow, Result};
use futures_util::sink::SinkExt;
use log::{debug, info};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::broadcast::{self, Receiver as BroadcastReceiver};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout, Duration};
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};
use uuid::Uuid;

static CWS_EVENT_SENDER: std::sync::OnceLock<UnboundedSender<Value>> = std::sync::OnceLock::new();
static WS_EVENT_SENDER: std::sync::OnceLock<UnboundedSender<Value>> = std::sync::OnceLock::new();

#[derive(Clone)]
struct WsEventHub {
    broadcaster: broadcast::Sender<Value>,
    last_events: Arc<Mutex<HashMap<String, Value>>>,
}

impl WsEventHub {
    fn new(buffer: usize) -> Self {
        let (tx, _) = broadcast::channel(buffer);
        Self {
            broadcaster: tx,
            last_events: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn subscribe(&self) -> BroadcastReceiver<Value> {
        self.broadcaster.subscribe()
    }

    async fn push(&self, event: Value) {
        if let Some(order_id) = order_id_from_ws(&event) {
            let mut map = self.last_events.lock().await;
            map.insert(order_id, event.clone());
        }
        let _ = self.broadcaster.send(event);
    }

    async fn latest_for(&self, order_id: &str) -> Option<Value> {
        let map = self.last_events.lock().await;
        map.get(order_id).cloned()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    init_logger("INFO");

    let (cws_event_tx, mut cws_event_rx) = unbounded_channel();
    let (ws_event_tx, ws_event_rx) = unbounded_channel();

    CWS_EVENT_SENDER
        .set(cws_event_tx)
        .map_err(|_| anyhow!("Failed to set CWS event sender"))?;
    WS_EVENT_SENDER
        .set(ws_event_tx)
        .map_err(|_| anyhow!("Failed to set WS event sender"))?;

    let ws_hub = WsEventHub::new(256);

    // Единственный роутер читает все WS события и раздает через broadcast
    tokio::spawn(ws_router(ws_event_rx, ws_hub.clone()));

    // Инициализируем клиента
    let mut client = AlorRust::new("f4284595-00ba-43c9-a304-68f53b6b4b6a", false, log_ws_event, log_cws_event).await?;

    // Конфигурация, сопоставимая с Python-скриптом
    let portfolio = std::env::var("ALOR_PORTFOLIO").unwrap_or_else(|_| "7502T0U".to_string());
    let symbol = std::env::var("ALOR_SYMBOL").unwrap_or_else(|_| "IMOEXF".to_string());
    let exchange = std::env::var("ALOR_EXCHANGE").unwrap_or_else(|_| "MOEX".to_string());
    let qty: i32 = std::env::var("ALOR_QTY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let price: f64 = std::env::var("ALOR_PRICE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2700.0);
    let update_delta: f64 = std::env::var("ALOR_UPDATE_DELTA")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10.0);
    let allow_margin = std::env::var("ALOR_ALLOW_MARGIN")
        .ok()
        .map(|v| v.to_lowercase() == "true")
        .unwrap_or(true);
    let time_in_force = std::env::var("ALOR_TIME_IN_FORCE").unwrap_or_else(|_| "BookOrCancel".to_string());
    let tif = match time_in_force.as_str() {
        "GoodTillCancelled" => Some(TimeInForce::GoodTillCancelled),
        _ => Some(TimeInForce::BookOrCancel),
    };

    // Явное получение токена (без скрытых дополнительных запросов)
    let token_started = Instant::now();
    let token_json: Value = serde_json::from_str(
        client
            .auth_client
            .get_jwt_token_force()
            .await?
            .as_str(),
    )?;
    info!(
        "[TIMING] get_access_token_ms: {} ms",
        token_started.elapsed().as_millis()
    );
    let access_token = token_json
        .get("AccessToken")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("AccessToken not found in JWT response"))?
        .to_string();

    // Подписываемся на статусы ордеров (с ожиданием ack/первого сообщения)
    let (subscribe_guid, subscribe_ms) = subscribe_order_statuses(
        &mut client,
        &ws_hub,
        &access_token,
        &portfolio,
        &exchange,
    )
    .await?;
    info!(
        "<< SUBSCRIBE FIRST: guid={}, dt={} ms",
        subscribe_guid, subscribe_ms
    );

    // Разогрев/проверка позиций до начала измерений create/update/delete
    let _ = client
        .get_positions(&portfolio, &exchange, true, "Simple")
        .await?;

    // CREATE
    let mut ws_rx_for_create = ws_hub.subscribe();
    let create_send = Instant::now();
    let create_guid = client
        .cws_client
        .create_limit_order(
            OrderSide::Buy,
            qty,
            price,
            &symbol,
            &exchange,
            None,
            &portfolio,
            None,
            tif,
            Some(allow_margin),
            None,
            None,
            Some(true),
        )
        .await?;
    let (create_cws_ack, create_ack_ms) =
        wait_for_cws_by_guid(&mut cws_event_rx, &create_guid, create_send).await?;
    let order_number = order_id_from_cws(&create_cws_ack)
        .ok_or_else(|| anyhow!("No orderNumber in create response"))?;

    let (create_ws_evt, create_ws_ms) = wait_ws_event_for_order(
        &ws_hub,
        &mut ws_rx_for_create,
        &order_number,
        create_send,
        Duration::from_secs(2),
        None,
    )
    .await
    .ok_or_else(|| anyhow!("No WS event for created order"))?;

    info!("\n========== AFTER CREATE ==========");
    info!("orderNumber (ack): {}", order_number);
    info!("[TIMING] create_ack_ms           : {} ms", create_ack_ms);
    info!(
        "[TIMING] create_to_first_ws_ms  : {} ms",
        create_ws_ms
    );
    info!(
        "[TIMING] wait_first_ws_ms       : {} ms",
        create_ws_ms.saturating_sub(create_ack_ms)
    );
    log_ws_payload(&create_ws_evt, "stream");

    // UPDATE
    let new_price = price + update_delta;
    let old_order_number = order_number.clone();
    let mut ws_rx_for_update = ws_hub.subscribe();
    let update_send = Instant::now();
    let update_guid = client
        .cws_client
        .update_limit_order(
            &order_number,
            OrderSide::Buy,
            qty,
            new_price,
            &symbol,
            &exchange,
            None,
            &portfolio,
            None,
            Some(allow_margin),
            None,
            Some(true),
        )
        .await?;
    let (update_ack_evt, update_ack_ms) =
        wait_for_cws_by_guid(&mut cws_event_rx, &update_guid, update_send).await?;
    let new_order_number = order_id_from_cws(&update_ack_evt).unwrap_or_else(|| order_number.clone());

    let (old_ws_evt, old_ws_ms) = wait_ws_event_for_order(
        &ws_hub,
        &mut ws_rx_for_update,
        &old_order_number,
        update_send,
        Duration::from_secs(2),
        Some(|status| status == "canceled" || status == "cancelled"),
    )
    .await
    .ok_or_else(|| anyhow!("No WS cancel event for old order"))?;

    let (new_ws_evt, new_ws_ms) = wait_ws_event_for_order(
        &ws_hub,
        &mut ws_rx_for_update,
        &new_order_number,
        update_send,
        Duration::from_secs(2),
        None,
    )
    .await
    .ok_or_else(|| anyhow!("No WS event for new order"))?;

    info!("\n========== AFTER UPDATE ==========");
    info!("orderNumber (ack/new): {}", new_order_number);
    info!("[TIMING] update_ack_ms         : {} ms", update_ack_ms);
    info!(
        "[TIMING] wait_old_cancel_ms    : {} ms",
        old_ws_ms.saturating_sub(update_ack_ms)
    );
    info!(
        "[TIMING] wait_new_evt_ms       : {} ms",
        new_ws_ms.saturating_sub(old_ws_ms)
    );

    info!("\nOld Order (before update):");
    info!("orderNumber: {}", old_order_number);
    log_ws_payload(&old_ws_evt, "Old Order");

    info!("\nNew Order (after update):");
    info!("orderNumber: {}", new_order_number);
    log_ws_payload(&new_ws_evt, "New Order");
    if order_id_from_ws(&new_ws_evt).as_deref() != Some(&new_order_number) {
        info!("Warning: WS id does not match new orderNumber");
    }

    // DELETE
    let mut ws_rx_for_delete = ws_hub.subscribe();
    let delete_send = Instant::now();
    let delete_guid = client
        .cws_client
        .delete_limit_order(&new_order_number, &exchange, &portfolio, Some(true))
        .await?;
    let (_delete_ack_evt, delete_ack_ms) =
        wait_for_cws_by_guid(&mut cws_event_rx, &delete_guid, delete_send).await?;

    let (delete_ws_evt, delete_ws_ms) = wait_ws_event_for_order(
        &ws_hub,
        &mut ws_rx_for_delete,
        &new_order_number,
        delete_send,
        Duration::from_secs(2),
        Some(is_terminal_status),
    )
    .await
    .ok_or_else(|| anyhow!("No WS delete event for order"))?;

    info!("\n========== AFTER DELETE ==========");
    info!("[TIMING] delete_ack_ms         : {} ms", delete_ack_ms);
    info!(
        "[TIMING] wait_delete_evt_ms    : {} ms",
        delete_ws_ms.saturating_sub(delete_ack_ms)
    );
    log_ws_payload(&delete_ws_evt, "stream");

    sleep(Duration::from_secs(1)).await;
    Ok(())
}

async fn subscribe_order_statuses(
    client: &mut AlorRust,
    ws_hub: &WsEventHub,
    access_token: &str,
    portfolio: &str,
    exchange: &str,
) -> Result<(Uuid, u128)> {
    let subscribe_guid = Uuid::new_v4();
    let mut request_body = Value::Object(serde_json::Map::new());
    if let Some(body) = request_body.as_object_mut() {
        body.insert(
            "opcode".to_string(),
            Value::from("OrdersGetAndSubscribeV2"),
        );
        body.insert("exchange".to_string(), Value::from(exchange));
        body.insert("portfolio".to_string(), Value::from(portfolio));
        body.insert("skipHistory".to_string(), Value::from(true));
        body.insert("frequency".to_string(), Value::from(0));
        body.insert("format".to_string(), Value::from("Simple"));
        body.insert("token".to_string(), Value::from(access_token));
        body.insert("guid".to_string(), Value::from(subscribe_guid.to_string()));
    }

    let message = Message::Text(Utf8Bytes::from(request_body.to_string()));
    let mut subscribe_rx = ws_hub.subscribe();
    let subscribe_started = Instant::now();
    info!(
        "Sending OrdersGetAndSubscribeV2, guid={}, portfolio={}, exchange={}",
        subscribe_guid, portfolio, exchange
    );
    client
        .client
        .socket_client
        .write_stream
        .send(message)
        .await?;

    let ack_evt = wait_ws_subscribe_ack(
        &mut subscribe_rx,
        &subscribe_guid,
        subscribe_started,
        Duration::from_secs(5),
    )
    .await?;
    let roundtrip_ms = subscribe_started.elapsed().as_millis();
    debug!("Subscribe ack/data: {:?}", ack_evt);
    Ok((subscribe_guid, roundtrip_ms))
}

async fn wait_ws_subscribe_ack(
    rx: &mut BroadcastReceiver<Value>,
    guid: &Uuid,
    started: Instant,
    timeout_duration: Duration,
) -> Result<Value> {
    loop {
        let remaining = timeout_duration.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(anyhow!("Timeout waiting for subscribe ack"));
        }

        let evt = match timeout(remaining, rx.recv()).await {
            Ok(Ok(evt)) => evt,
            Ok(Err(e)) => return Err(anyhow!("Subscribe channel closed: {}", e)),
            Err(_) => return Err(anyhow!("Timeout waiting for subscribe ack")),
        };

        let guid_match_top = evt
            .get("guid")
            .and_then(|v| v.as_str())
            .map(|g| g == guid.to_string())
            .unwrap_or(false);
        let guid_match_data = evt
            .get("data")
            .and_then(|d| d.get("guid"))
            .and_then(|v| v.as_str())
            .map(|g| g == guid.to_string())
            .unwrap_or(false);
        let http_ok = evt
            .get("httpCode")
            .and_then(|v| v.as_u64())
            .map(|code| code == 200)
            .unwrap_or(false);

        if guid_match_top || guid_match_data {
            return Ok(evt);
        }
        if http_ok {
            return Ok(evt);
        }
        debug!("Skipping non-subscribe event while waiting: {}", evt);
    }
}

fn order_id_from_ws(event: &Value) -> Option<String> {
    // Simple формат: data.id
    if let Some(data) = event.get("data") {
        if let Some(arr) = data.as_array() {
            for item in arr {
                if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                    return Some(id.to_string());
                }
            }
        }
        if let Some(obj) = data.as_object() {
            if let Some(id) = obj.get("id").and_then(|v| v.as_str()) {
                return Some(id.to_string());
            }
        }
    }
    if let Some(id) = event.get("id").and_then(|v| v.as_str()) {
        return Some(id.to_string());
    }
    None
}

fn order_id_from_cws(event: &Value) -> Option<String> {
    event.get("orderNumber").and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn status_from_ws(event: &Value) -> Option<String> {
    if let Some(data) = event.get("data") {
        if let Some(arr) = data.as_array() {
            for item in arr {
                if let Some(status) = item.get("status").and_then(|v| v.as_str()) {
                    return Some(status.to_lowercase());
                }
            }
        }
        if let Some(obj) = data.as_object() {
            if let Some(status) = obj.get("status").and_then(|v| v.as_str()) {
                return Some(status.to_lowercase());
            }
        }
    }
    event
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| s.to_lowercase())
}

fn is_terminal_status(status: &str) -> bool {
    matches!(
        status,
        "canceled"
            | "cancelled"
            | "rejected"
            | "filled"
            | "expired"
            | "done"
            | "completed"
    )
}

async fn wait_ws_event_for_order(
    hub: &WsEventHub,
    rx: &mut BroadcastReceiver<Value>,
    order_id: &str,
    start: Instant,
    timeout_duration: Duration,
    status_predicate: Option<fn(&str) -> bool>,
) -> Option<(Value, u128)> {
    if let Some(ev) = hub.latest_for(order_id).await {
        if status_predicate
            .map(|pred| status_from_ws(&ev).as_deref().map(pred).unwrap_or(false))
            .unwrap_or(true)
        {
            return Some((ev, start.elapsed().as_millis() as u128));
        }
    }

    loop {
        let remaining = timeout_duration.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return None;
        }
        let recv_result = timeout(remaining, rx.recv()).await.ok()?;
        let evt = recv_result.ok()?;
        if let Some(id) = order_id_from_ws(&evt) {
            if id != order_id {
                continue;
            }
            if let Some(pred) = status_predicate {
                if let Some(status) = status_from_ws(&evt) {
                    if !pred(&status) {
                        continue;
                    }
                } else {
                    continue;
                }
            }
            let elapsed_ms = start.elapsed().as_millis();
            return Some((evt, elapsed_ms as u128));
        }
    }
}

async fn wait_for_cws_by_guid(
    rx: &mut UnboundedReceiver<Value>,
    guid: &str,
    start: Instant,
) -> Result<(Value, u128)> {
    while let Some(evt) = rx.recv().await {
        if evt
            .get("requestGuid")
            .and_then(|v| v.as_str())
            .map(|g| g == guid)
            .unwrap_or(false)
        {
            let elapsed = start.elapsed().as_millis();
            return Ok((evt, elapsed as u128));
        }
    }
    Err(anyhow!("CWS event channel closed"))
}

async fn ws_router(mut rx: UnboundedReceiver<Value>, hub: WsEventHub) {
    while let Some(event) = rx.recv().await {
        hub.push(event).await;
    }
}

fn log_ws_event(event: &Value) {
    if let Some(sender) = WS_EVENT_SENDER.get() {
        let _ = sender.send(event.clone());
    }
}

fn log_cws_event(event: &Value) {
    if let Some(sender) = CWS_EVENT_SENDER.get() {
        let _ = sender.send(event.clone());
    }
}

fn log_ws_payload(event: &Value, label: &str) {
    let status = status_from_ws(event).unwrap_or_else(|| "<unknown>".to_string());
    info!("{} status: {}", label, status);
    info!("{} rec   : {}", label, event);
}