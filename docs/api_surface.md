# API Surface (Current)

Краткая карта публичного API `alor_rust` на текущем этапе.

## Основной тип

- `AlorRust`

Публичные entry points:

- `AlorRust::new(refresh_token, demo, ws_callback, cws_callback)`
- `AlorRust::new_default_callbacks(refresh_token, demo)`
- `AlorRust::builder(refresh_token)`
- `AlorRust::from_config(refresh_token, AlorConfig)`

## Config / Builder

- `AlorConfig`
- `AlorClientBuilder`

Поддержаны поля/настройки:

- `demo`
- `ws_callback`
- `cws_callback`
- `enable_ws` (подготовка, пока `false` не поддержан)
- `enable_cws` (подготовка, пока `false` не поддержан)
- `preload_jwt`

## REST / Utility методы (`AlorRust`)

- `get_positions(...)`
- `get_history(...)`
- `get_symbol(...)`
- `get_symbol_info(...)`
- `get_server_time()`
- `subscribe_bars(...)`
- `dataname_to_board_symbol(...)`
- `get_exchange(...)`
- `utc_timestamp_to_msk_datetime(...)`
- `msk_datetime_to_utc_timestamp(...)`
- `get_account(...)`
- `accounts()`

## WS / CWS event streams helpers

- `subscribe_ws_events()`
- `subscribe_cws_events()`

Ожидание/поиск событий:

- `wait_cws_event_by_request_guid(...)`
- `wait_ws_event_by_guid(...)`
- `wait_ws_order_status_by_id(...)`
- `wait_ws_order_status_by_id_and_predicate(...)`
- `wait_ws_order_status_event(...)`

Парсеры полей:

- `cws_request_guid(...)`
- `cws_order_number(...)`
- `cws_http_code(...)`
- `ws_event_guid(...)`
- `ws_order_status_id(...)`
- `ws_order_status(...)`

## Подписка на статусы ордеров (`OrdersGetAndSubscribeV2`)

- `subscribe_orders_statuses_v2(...)`
- `subscribe_orders_statuses_v2_and_wait_ack(...)`
- `subscribe_orders_statuses_v2_and_wait_ack_typed(...)`

## High-level limit order flow helpers

- `create_limit_order_and_wait_status_id(...)`
- `update_limit_order_and_wait_status(...)`
- `delete_limit_order_and_wait_status(...)`

## Низкоуровневые CWS команды (`client.cws_client`)

Поддержаны create/update/delete команды для:

- market
- limit
- stop
- stop-limit

Важно: низкоуровневые методы возвращают `requestGuid`, а не финальный `order_id`.

