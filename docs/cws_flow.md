# CWS Order Flow (Cookbook)

Документирует рекомендуемый на текущий момент сценарий для limit order:

- subscribe statuses (`OrdersGetAndSubscribeV2`)
- create -> wait ack/status/id
- update -> wait ack/status(s)
- delete -> wait ack/status

Ключевая идея:

- низкоуровневые CWS методы возвращают `requestGuid`
- фактический `order_id` берется из WS-статусов (`OrdersGetAndSubscribeV2`, поле `data.id`)

## Источник истины для ID заявки

Использовать `WS` событие статуса (`OrdersGetAndSubscribeV2`) и поле:

- `data.id`

`CWS ack.orderNumber` полезен как ранний ориентир/сопоставление, но финальный рабочий flow должен опираться на поток статусов.

## Live-подтвержденный сценарий

Проверено на smoke test `examples/cws_orders_smoke.rs`:

- `create:limit` -> `CWS ack` -> `WS status (working)`
- `update:limit` -> `CWS ack` (новый `orderNumber`) -> `WS old=canceled` + `WS new=working`
- `delete:limit` -> `CWS ack` -> `WS status (canceled)`

В проверенном кейсе `update:limit` работает как:

- `cancel old + create new`

## Рекомендуемый код (current API)

```rust
let mut ws_rx = client.subscribe_ws_events();
let mut cws_rx = client.subscribe_cws_events();

let (sub_guid, _sub_ack) = client
    .subscribe_orders_statuses_v2_and_wait_ack_typed(
        &mut ws_rx,
        "MOEX",
        portfolio,
        None,
        true,
        0,
        "Simple",
        std::time::Duration::from_secs(5),
    )
    .await?;

let create = client
    .create_limit_order_and_wait_status_id(
        &mut cws_rx,
        &mut ws_rx,
        &sub_guid.to_string(),
        OrderSide::Buy,
        1,
        2700.0,
        "IMOEXF",
        "MOEX",
        None,
        portfolio,
        None,
        Some(TimeInForce::BookOrCancel),
        Some(true),
        None,
        None,
        Some(true),
        std::time::Duration::from_secs(5),
    )
    .await?;

let update = client
    .update_limit_order_and_wait_status(
        &mut cws_rx,
        &mut ws_rx,
        &sub_guid.to_string(),
        &create.order_id,
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
        Some(true),
        std::time::Duration::from_secs(5),
    )
    .await?;

let _delete = client
    .delete_limit_order_and_wait_status(
        &mut cws_rx,
        &mut ws_rx,
        &update.new_order_id,
        "MOEX",
        portfolio,
        Some(true),
        std::time::Duration::from_secs(5),
    )
    .await?;
```

## Helper-методы, которые участвуют в flow

- `subscribe_orders_statuses_v2_and_wait_ack_typed(...)`
- `create_limit_order_and_wait_status_id(...)`
- `update_limit_order_and_wait_status(...)`
- `delete_limit_order_and_wait_status(...)`

Низкоуровневые (если нужен свой flow):

- `wait_cws_event_by_request_guid(...)`
- `wait_ws_event_by_guid(...)`
- `wait_ws_order_status_by_id(...)`
- `wait_ws_order_status_event(...)`

