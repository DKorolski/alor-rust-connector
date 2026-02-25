# alor-rust-connector (`alor_rust`)

Локальная Rust-библиотека для работы с API Alor:

- REST market data / справочные методы
- WebSocket подписка на бары (`/ws`)
- Command WebSocket для торговых команд (`/cws`)

Важно: это описание составлено по текущему коду в репозитории (фактическое состояние), а не по планируемому API.

## Текущее состояние (честно)

Проект находится в рабочем, но сыром состоянии:

- публичный API есть и покрывает базовые REST/CWS операции;
- документация была минимальной и частично устаревшей;
- в коде много `unwrap/expect/panic` (ошибки не везде обрабатываются безопасно);
- примеры были приведены в порядок, но набор пока минимальный (1 live smoke test);
- автоматических тестов сейчас нет.

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

Полностью типизированный сценарий "create/update/delete + wait statuses" пока не завершен, но уже есть первый helper высокого уровня для create-flow: `create_limit_order_and_wait_status_id(...)`.

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
- `cws_request_guid(...)`
- `cws_order_number(...)`
- `ws_order_status_id(...)`

Это позволяет собрать flow без доступа к внутренним полям `write_stream/read_stream`.

## Примеры (`examples/`)

В каталоге оставлен один рекомендуемый пример:

1. `examples/cws_orders_smoke.rs`
- live smoke test на новом event-driven API (`subscribe_ws_events`, `subscribe_cws_events`);
- использует helper-методы высокого уровня (`create_limit_order_and_wait_status_id`, `delete_limit_order_and_wait_status`);
- подтверждает сценарий create -> WS `data.id` -> delete через `OrdersGetAndSubscribeV2`.

## Конфигурация и переменные окружения

### Обязательное для работы

- refresh token передается в `AlorRust::new(...)`

### Важно: скрытая зависимость текущего кода

В текущей реализации разбор аккаунтов из JWT внутри `AuthClient` использует переменную окружения `PORTFOLIO_NUMBER`.

Если она не задана, инициализация/обновление токена может завершиться panic.

Пример:

```bash
export PORTFOLIO_NUMBER=750
```

Это ограничение текущей реализации, планируется убрать/сделать явным параметром.

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

- много `unwrap/expect/panic` в runtime путях;
- нет нормализованного error type;
- нет удобного публичного API ожидания CWS response по `requestGuid`;
- сообщения CWS без `requestGuid` обрабатываются неполно;
- нет reconnect/resubscribe логики;
- нет тестов и CI;
- `cargo check` в офлайн-окружении не проходит без доступа к `crates.io` (если зависимости не закэшированы).

## Что дальше

План улучшений по итерациям и критерии приемки см. в:

- `alor-rust-connector/docs/AUDIT_AND_ITERATION_TZ.md`
