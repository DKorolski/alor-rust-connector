# alor-rust-connector (`alor_rust`)

Локальная Rust-библиотека для работы с API Alor:

- REST market data / справочные методы
- WebSocket подписка на бары (`/ws`)
- Command WebSocket для торговых команд (`/cws`)

Важно: это описание составлено по текущему коду в репозитории (фактическое состояние), а не по планируемому API.

## Текущее состояние (честно)

Проект находится в рабочем, но сыром состоянии:

- публичный API есть и покрывает базовые REST/CWS операции;
- документация актуализирована под фактический код, но ещё требует финальной полировки;
- основные runtime `unwrap/expect/panic` в `src/` вычищены (остатки допустимы в тестах/комментариях);
- примеры приведены в порядок, но набор пока минимальный (1 live smoke test);
- есть unit-тесты на helper-парсеры и часть config/builder логики.

Подробный аудит и план доработок: `alor-rust-connector/docs/AUDIT_AND_ITERATION_TZ.md`.

## Что есть в библиотеке сейчас

### Основной тип

`AlorRust` объединяет:

- `auth_client` (refresh/access token, разбор JWT, список аккаунтов/портфелей)
- `client` (REST + market-data websocket `/ws`)
- `cws_client` (торговый command websocket `/cws`)

Конструктор:

```rust
let mut client = AlorRust::new(
    refresh_token,
    demo,
    ws_callback,
    cws_callback,
).await?;
```

Также доступны более удобные entry points (старт Итерации 3):

```rust
// no-op callbacks по умолчанию
let client = AlorRust::new_default_callbacks(refresh_token, false).await?;

// builder / config style
let client = AlorRust::builder(refresh_token)
    .demo(false)
    .preload_jwt(true)
    .build()
    .await?;
```

Параметры:

- `refresh_token: &str` - refresh token Alor
- `demo: bool` - `true` для dev/demo серверов, `false` для prod
- `ws_callback: fn(&serde_json::Value)` - callback для market-data websocket
- `cws_callback: fn(&serde_json::Value)` - callback для CWS сообщений (ограничения см. ниже)

## REST / utility методы (`AlorRust`)

Сейчас в `src/lib.rs` доступны:

- `get_positions(...)`
- `get_history(...)`
- `get_symbol(...)`
- `get_symbol_info(...)`
- `get_server_time()`
- `subscribe_bars(...)` (отправка подписки в market-data websocket)
- `dataname_to_board_symbol(...)`
- `get_exchange(...)`
- `utc_timestamp_to_msk_datetime(...)` (название сейчас вводит в заблуждение)
- `msk_datetime_to_utc_timestamp(...)` (сигнатура сейчас неочевидная по смыслу)
- `get_account(...)`
- `accounts()`

## Торговые команды CWS (`client.cws_client`)

Поддержаны методы отправки команд:

- `create_market_order`
- `create_limit_order`
- `create_stop_order`
- `create_stop_limit_order`
- `update_market_order`
- `update_limit_order`
- `update_stop_order`
- `update_stop_limit_order`
- `delete_market_order`
- `delete_limit_order`
- `delete_stop_order`
- `delete_stop_limit_order`

### Важное ограничение текущего API CWS

Эти методы сейчас возвращают `Result<String>`, где строка - это `requestGuid` (GUID запроса), а не ответ сервера и не `orderNumber`.

Для limit order уже реализован рабочий сценарий высокого уровня с ожиданием статусов:
- `create_limit_order_and_wait_status_id(...)`
- `update_limit_order_and_wait_status(...)`
- `delete_limit_order_and_wait_status(...)`

### Что уже добавлено для нормального сценария (event streams)

Для построения штатного сценария теперь можно использовать внутренние event-stream подписки:

- `subscribe_ws_events()` - поток всех WS JSON-событий (включая `OrdersGetAndSubscribeV2`)
- `subscribe_cws_events()` - поток всех CWS JSON-событий

И helper-методы:

- `wait_cws_event_by_request_guid(...)`
- `wait_ws_event_by_guid(...)`
- `wait_ws_order_status_by_id(...)`
- `wait_ws_order_status_event(...)`
- `create_limit_order_and_wait_status_id(...)`
- `update_limit_order_and_wait_status(...)`
- `delete_limit_order_and_wait_status(...)`
- `subscribe_orders_statuses_v2_and_wait_ack(...)`
- `subscribe_orders_statuses_v2_and_wait_ack_typed(...)`
- `cws_request_guid(...)`
- `cws_order_number(...)`
- `ws_order_status_id(...)`

Это позволяет собрать flow без доступа к внутренним полям `write_stream/read_stream`.

### Typed события (новый слой API)

Для helper-методов высокого уровня добавлены типизированные события-обёртки:

- `CwsAckEvent`
- `WsOrderStatusEvent`
- `WsSubscribeAckEvent`

Они содержат:

- `raw: serde_json::Value` (исходный JSON)
- основные распарсенные поля (`http_code`, `request_guid`, `order_number`, `order_id`, `status`, и т.д.)

Это позволяет постепенно уходить от ручного парсинга `Value`, не ломая обратную совместимость логов/примеров.

### Что уже подтверждено live (через `cws_orders_smoke`)

- `OrdersGetAndSubscribeV2` отдает статусы заявок и `data.id` (источник фактического ID заявки)
- `create:limit` -> `CWS ack` -> `WS status (working)`
- `update:limit` -> `CWS ack` с новым `orderNumber` -> `WS old=canceled` + `WS new=working`
- `delete:limit` -> `CWS ack` -> `WS status (canceled)`

Это означает, что на текущем проверенном сценарии update действительно реализуется как `cancel old + create new`.

### Рекомендуемый Flow (Текущий API)

```rust
let mut ws_rx = client.subscribe_ws_events();
let mut cws_rx = client.subscribe_cws_events();

let (sub_guid, _sub_ack) = client
    .subscribe_orders_statuses_v2_and_wait_ack_typed(
        &mut ws_rx, "MOEX", portfolio, None, true, 0, "Simple",
        std::time::Duration::from_secs(5),
    )
    .await?;

let create = client
    .create_limit_order_and_wait_status_id(
        &mut cws_rx, &mut ws_rx, &sub_guid.to_string(),
        OrderSide::Buy, 1, 2700.0, "IMOEXF", "MOEX", None, portfolio,
        None, Some(TimeInForce::BookOrCancel), Some(true), None, None, Some(true),
        std::time::Duration::from_secs(5),
    )
    .await?;

let update = client
    .update_limit_order_and_wait_status(
        &mut cws_rx, &mut ws_rx, &sub_guid.to_string(), &create.order_id,
        OrderSide::Buy, 1, 2710.0, "IMOEXF", "MOEX", None, portfolio,
        None, Some(true), None, Some(true), std::time::Duration::from_secs(5),
    )
    .await?;

let _delete = client
    .delete_limit_order_and_wait_status(
        &mut cws_rx, &mut ws_rx, &update.new_order_id, "MOEX", portfolio,
        Some(true), std::time::Duration::from_secs(5),
    )
    .await?;
```

## Примеры (`examples/`)

В каталоге оставлен один рекомендуемый пример:

1. `examples/cws_orders_smoke.rs`
- live smoke test на новом event-driven API (`subscribe_ws_events`, `subscribe_cws_events`);
- использует typed helper подписки `subscribe_orders_statuses_v2_and_wait_ack_typed(...)`;
- использует helper-методы высокого уровня (`create_limit_order_and_wait_status_id`, `update_limit_order_and_wait_status`, `delete_limit_order_and_wait_status`);
- подтверждает сценарий create -> update -> delete с получением ID из `OrdersGetAndSubscribeV2`.

## Конфигурация и переменные окружения

### Новый конфиг (старт Итерации 3)

Добавлены:

- `AlorConfig`
- `AlorClientBuilder`
- `AlorRust::from_config(...)`

Это позволяет централизованно задавать `demo`, callbacks и поведение preload JWT.

Примечание: поля `enable_ws` / `enable_cws` уже добавлены в конфиг как подготовка к `REST-only`/lazy init, но их выключение пока возвращает явную ошибку (режим еще не реализован).

### Обязательное для работы

- refresh token передается в `AlorRust::new(...)`, `AlorRust::new_default_callbacks(...)` или `AlorRust::builder(...)`

### Примечание по `PORTFOLIO_NUMBER`

Переменная `PORTFOLIO_NUMBER` больше не является обязательной (panic-path убран).
Если задана, используется как дополнительная фильтрация при разборе деривативных портфелей из JWT.

## Серверы (demo/prod)

В `src/helpers/servers.rs` зашиты URL:

- demo:
- `https://oauthdev.alor.ru`
- `https://apidev.alor.ru`
- `wss://apidev.alor.ru/ws`
- `wss://apidev.alor.ru/cws`

- prod:
- `https://oauth.alor.ru`
- `https://api.alor.ru`
- `wss://api.alor.ru/ws`
- `wss://api.alor.ru/cws`

## Минимальный пример (текущий стиль API)

```rust
use alor_rust::*;
use serde_json::Value;
use anyhow::Result;

fn on_ws(_event: &Value) {}
fn on_cws(_event: &Value) {}

#[tokio::main]
async fn main() -> Result<()> {
    init_logger("INFO");

    let mut client = AlorRust::new(
        "YOUR_REFRESH_TOKEN",
        true,
        on_ws,
        on_cws,
    ).await?;

    let server_time = client.get_server_time().await?;
    println!("server time: {}", server_time);

    Ok(())
}
```

## Известные ограничения (на февраль 2026, по локальному коду)

- единый error type для всей библиотеки ещё не завершён (используется `anyhow::Result`, но есть исторические участки/контракты, которые ещё можно унифицировать лучше);
- `AlorRust` пока поднимает и `ws`, и `cws` сразу (REST-only/lazy init ещё не реализованы);
- `enable_ws/enable_cws` в `AlorConfig` пока только подготовлены и при `false` возвращают явную ошибку;
- нет reconnect/resubscribe логики;
- публичные поля `AlorRust`/части внутренних структур ещё слишком открыты (эргономика/инкапсуляция будут улучшаться);
- CI ещё не добавлен (`cargo fmt/clippy/test` pipeline отсутствует).

## Что дальше

План улучшений по итерациям и критерии приемки см. в:

- `alor-rust-connector/docs/AUDIT_AND_ITERATION_TZ.md`
