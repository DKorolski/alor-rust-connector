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
- примеры в `examples/` не все отражают рабочий production-flow;
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

То есть сценарий "создать заявку -> получить идентификатор заявки (`orderNumber`/`data.id`) через `OrdersGetAndSubscribeV2` -> изменить/удалить" пока не оформлен как удобный публичный API.

## Примеры (`examples/`)

В каталоге есть 3 примера:

1. `examples/cws_example.rs`
- большой демонстрационный сценарий create/update/delete для разных типов заявок;
- содержит хардкод токена/портфелей и не подходит как безопасный шаблон.

2. `examples/cws_example_1.rs`
- пытается сделать create -> прочитать событие -> извлечь `orderNumber` -> update/delete;
- расходится с текущим API `CWS` (обращается к полю `read_stream`, которого нет).

3. `examples/cws_example_2.rs`
- упрощенный сценарий для limit order;
- использует фиктивный `order_number = "10"` и поэтому не является полноценным рабочим примером.

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
