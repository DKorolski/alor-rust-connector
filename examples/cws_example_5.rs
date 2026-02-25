use alor_rust::dto::cws_dto::order_common::{OrderSide, TimeInForce};
use alor_rust::*;
use anyhow::{anyhow, Result};
use log::{debug, info, warn};
use serde_json::Value;
use tokio::sync::broadcast;
use tokio::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    let log_level = std::env::var("RUST_LOG").unwrap_or_else(|_| "INFO".to_string());
    init_logger(&log_level);

    let refresh_token =
        std::env::var("ALOR_REFRESH_TOKEN").map_err(|_| anyhow!("Set ALOR_REFRESH_TOKEN"))?;
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

    let mut client = AlorRust::new(&refresh_token, false, log_ws_event, log_cws_event).await?;

    let mut ws_rx = client.subscribe_ws_events();
    let mut cws_rx = client.subscribe_cws_events();

    let subscribe_guid = client
        .subscribe_orders_statuses_v2(&exchange, &portfolio, None, true, 0, "Simple")
        .await
        .map_err(|e| anyhow!(e.to_string()))?;
    let subscribe_guid_s = subscribe_guid.to_string();
    let subscribe_evt = AlorRust::wait_ws_event_by_guid(
        &mut ws_rx,
        &subscribe_guid_s,
        Duration::from_secs(1),
    )
    .await;
    let subscribe_evt = match subscribe_evt {
        Ok(evt) => evt,
        Err(_) => wait_ws_subscribe_ack(&mut ws_rx, &subscribe_guid_s, Duration::from_secs(5)).await?,
    };
    info!("OrdersGetAndSubscribeV2 ack/event: {}", subscribe_evt);

    // create
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
            Some(TimeInForce::BookOrCancel),
            Some(true),
            None,
            None,
            Some(true),
        )
        .await?;
    let create_ack = AlorRust::wait_cws_event_by_request_guid(
        &mut cws_rx,
        &create_guid,
        Duration::from_secs(5),
    )
    .await
    .map_err(|e| anyhow!(e.to_string()))?;
    info!("CWS create ack: {}", create_ack);

    let ws_create_evt = wait_order_status_event(
        &mut ws_rx,
        &subscribe_guid_s,
        &portfolio,
        &symbol,
        None,
        Duration::from_secs(5),
    )
    .await?;
    info!("WS create status event: {}", ws_create_evt);

    // Источник истины по ID заявки: событие статуса (OrdersGetAndSubscribeV2 -> data.id)
    let order_id = AlorRust::ws_order_status_id(&ws_create_evt)
        .or_else(|| AlorRust::cws_order_number(&create_ack))
        .ok_or_else(|| anyhow!("No order id in WS status event or CWS ack"))?;
    info!("Order id (source-of-truth WS): {}", order_id);

    // delete (smoke test)
    let delete_guid = client
        .cws_client
        .delete_limit_order(&order_id, &exchange, &portfolio, Some(true))
        .await?;
    let delete_ack = AlorRust::wait_cws_event_by_request_guid(
        &mut cws_rx,
        &delete_guid,
        Duration::from_secs(5),
    )
    .await
    .map_err(|e| anyhow!(e.to_string()))?;
    info!("CWS delete ack: {}", delete_ack);

    let ws_delete_evt = wait_order_status_event(
        &mut ws_rx,
        &subscribe_guid_s,
        &portfolio,
        &symbol,
        Some(&order_id),
        Duration::from_secs(5),
    )
    .await?;
    info!("WS delete status event: {}", ws_delete_evt);

    if let Some(status) = ws_status(&ws_delete_evt) {
        info!("Delete status: {}", status);
    }

    Ok(())
}

async fn wait_order_status_event(
    rx: &mut broadcast::Receiver<Value>,
    subscribe_guid: &str,
    portfolio: &str,
    symbol: &str,
    order_id: Option<&str>,
    timeout_duration: Duration,
) -> Result<Value> {
    let fut = async {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if AlorRust::ws_event_guid(&event).as_deref() != Some(subscribe_guid) {
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

                    if let Some(expected_id) = order_id {
                        if AlorRust::ws_order_status_id(&event).as_deref() != Some(expected_id) {
                            continue;
                        }
                    }

                    return Ok(event);
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("WS receiver lagged by {} events", n);
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(anyhow!("WS broadcast stream closed"));
                }
            }
        }
    };

    tokio::time::timeout(timeout_duration, fut)
        .await
        .map_err(|_| anyhow!("Timeout waiting for order status event"))?
}

fn ws_status(event: &Value) -> Option<String> {
    event.get("data")
        .and_then(|d| d.get("status"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

async fn wait_ws_subscribe_ack(
    rx: &mut broadcast::Receiver<Value>,
    subscribe_guid: &str,
    timeout_duration: Duration,
) -> Result<Value> {
    let fut = async {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let top_guid_match =
                        AlorRust::ws_event_guid(&event).as_deref() == Some(subscribe_guid);
                    let data_guid_match = event
                        .get("data")
                        .and_then(|d| d.get("guid"))
                        .and_then(|v| v.as_str())
                        .map(|g| g == subscribe_guid)
                        .unwrap_or(false);
                    let request_guid_match = event
                        .get("requestGuid")
                        .and_then(|v| v.as_str())
                        .map(|g| g == subscribe_guid)
                        .unwrap_or(false);
                    let http_ok = event
                        .get("httpCode")
                        .and_then(|v| v.as_u64())
                        .map(|code| code == 200)
                        .unwrap_or(false);

                    if top_guid_match || data_guid_match || request_guid_match || http_ok {
                        return Ok(event);
                    }

                    debug!("Skipping WS event while waiting subscribe ack: {}", event);
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("WS receiver lagged by {} events while waiting subscribe ack", n);
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(anyhow!("WS broadcast stream closed"));
                }
            }
        }
    };

    tokio::time::timeout(timeout_duration, fut)
        .await
        .map_err(|_| anyhow!("Timeout waiting for OrdersGetAndSubscribeV2 ack"))?
}

fn log_ws_event(event: &Value) {
    debug!("WS event: {}", event);
}

fn log_cws_event(event: &Value) {
    debug!("CWS event: {}", event);
}
