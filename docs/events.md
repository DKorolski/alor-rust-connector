# Events and Typed Wrappers

Текущий event-driven слой строится вокруг JSON-событий (`serde_json::Value`) из:

- market-data WS (`/ws`)
- command WS (`/cws`)

Для ключевых сценариев добавлены typed wrappers с сохранением исходного JSON.

## Потоки событий

- `subscribe_ws_events()` -> `broadcast::Receiver<Value>`
- `subscribe_cws_events()` -> `broadcast::Receiver<Value>`

## Typed wrappers

### `CwsAckEvent`

Используется для ack-ответов CWS (`create/update/delete/...`).

Поля:

- `raw: Value`
- `http_code: Option<u64>`
- `message: Option<String>`
- `request_guid: Option<String>`
- `order_number: Option<String>`

Конструктор:

- `CwsAckEvent::from_raw(Value)`

### `WsOrderStatusEvent`

Используется для событий статусов заявок из `OrdersGetAndSubscribeV2`.

Поля:

- `raw: Value`
- `guid: Option<String>`
- `order_id: Option<String>`
- `status: Option<String>`
- `portfolio: Option<String>`
- `symbol: Option<String>`

Конструктор:

- `WsOrderStatusEvent::from_raw(Value)`

### `WsSubscribeAckEvent`

Используется для ack подписки на `OrdersGetAndSubscribeV2`.

Поля:

- `raw: Value`
- `guid: Option<String>`
- `request_guid: Option<String>`
- `http_code: Option<u64>`
- `message: Option<String>`
- `data_guid: Option<String>`

Конструктор:

- `WsSubscribeAckEvent::from_raw(Value)`

## Helper parsers (низкоуровневые)

Сохраняются для совместимости и фильтрации:

- `cws_request_guid(&Value)`
- `cws_order_number(&Value)`
- `cws_http_code(&Value)`
- `ws_event_guid(&Value)`
- `ws_order_status_id(&Value)`
- `ws_order_status(&Value)`

## Зачем wrappers, если есть `Value`

- удобнее читать код flow helper-ов
- меньше ручного парсинга JSON в прикладном коде
- сохраняется `raw` для логов / отладки / нестабильных полей
- можно постепенно перейти к более строгим typed DTO без ломки API

