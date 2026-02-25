# alor-rust-connector (`alor_rust`)

Локальная Rust-библиотека для работы с Alor API:

- REST (market data / reference)
- Market Data WebSocket (`/ws`)
- Command WebSocket (`/cws`) для торговых команд

Статус: рабочая, но развивающаяся библиотека. Есть live smoke test и подтвержденный event-driven flow для `create -> update -> delete` limit order.

## Текущее состояние

- Базовые REST/CWS операции доступны через `AlorRust`
- Event-driven helpers для `CWS + OrdersGetAndSubscribeV2` реализованы
- Typed wrappers для ключевых событий добавлены (`CwsAckEvent`, `WsOrderStatusEvent`, `WsSubscribeAckEvent`)
- Основные runtime `unwrap/expect/panic` в `src/` вычищены (остатки в тестах/комментариях)
- Есть unit-тесты на helper-парсеры / typed events / config-builder
- Начат config/builder слой (`AlorConfig`, `AlorClientBuilder`, `from_config`)

## Quickstart (live smoke)

```bash
cd alor-rust-connector

export ALOR_REFRESH_TOKEN="..."
# optional:
export ALOR_PORTFOLIO="..."
export ALOR_SYMBOL="IMOEXF"
export ALOR_EXCHANGE="MOEX"
export ALOR_QTY="1"
export ALOR_PRICE="2700"
export ALOR_UPDATE_DELTA="10"

RUST_LOG=info cargo run --example cws_orders_smoke
# debug:
# RUST_LOG=debug cargo run --example cws_orders_smoke
```

Примечание: вне активной торговой сессии сервер может вернуть `httpCode=400` с сообщением вида `Сейчас эта сессия не идет`.

## Минимальный пример

```rust
use alor_rust::*;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    init_logger("INFO");

    let mut client = AlorRust::new_default_callbacks(
        std::env::var("ALOR_REFRESH_TOKEN")?.as_str(),
        false, // demo=false -> prod
    )
    .await?;

    let t = client.get_server_time().await?;
    println!("server time: {t}");
    Ok(())
}
```

## Важное замечание по CWS (текущий API)

Низкоуровневые CWS-методы (`client.cws_client.create_* / update_* / delete_*`) возвращают `Result<String>`, где строка — это `requestGuid`, а не `orderNumber`.

Для нормального торгового flow используйте helpers + event streams:

- подписка на статусы заявок (`OrdersGetAndSubscribeV2`)
- `create/update/delete limit` с ожиданием подтвержденного `order_id`/статуса

Смотри рабочий пример: `examples/cws_orders_smoke.rs`

## Конфигурация

Поддержаны варианты создания клиента:

- `AlorRust::new(...)`
- `AlorRust::new_default_callbacks(...)`
- `AlorRust::builder(refresh_token) ... .build().await?`
- `AlorRust::from_config(...)`

`enable_ws/enable_cws` в `AlorConfig` уже добавлены, но `REST-only` / lazy init пока не реализованы.

## Ограничения (на февраль 2026)

- Нет reconnect/resubscribe логики
- `REST-only` / lazy init не завершены
- Единый error type не доведен до конца (в основном `anyhow::Result`)
- Публичные поля пока избыточно открыты
- CI (`fmt/clippy/test`) еще не добавлен

## Документация

- Аудит и план работ: `docs/AUDIT_AND_ITERATION_TZ.md`
- API surface (методы/entry points): `docs/api_surface.md`
- События и typed wrappers: `docs/events.md`
- CWS flow / cookbook: `docs/cws_flow.md`
- Troubleshooting: `docs/troubleshooting.md`

