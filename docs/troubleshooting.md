# Troubleshooting

Практические проблемы, которые уже встречались при live smoke test.

## `create_limit_order failed ... "Сейчас эта сессия не идет."`

Это не ошибка библиотеки.

Сервер Alor вернул `httpCode=400`, потому что торговая сессия для выбранного инструмента/рынка сейчас закрыта.

Что проверить:

- активна ли торговая сессия по выбранному инструменту;
- корректны ли `exchange` / `symbol` / `portfolio`;
- подходит ли `timeInForce` для текущего режима;
- корректна ли цена (особенно для `BookOrCancel`, когда заявка должна попасть в книгу).

Почему это теперь видно нормально:

- helper-методы возвращают `Err(...)` с полным `CWS ack`
- вместо “немого” таймаута видно реальный `httpCode` и сообщение сервера

## Ack подписки `OrdersGetAndSubscribeV2` не приходит по `guid`

На практике ack может прийти в формате с:

- `requestGuid`
- `httpCode=200`

Поэтому используйте:

- `subscribe_orders_statuses_v2_and_wait_ack(...)`
- или `subscribe_orders_statuses_v2_and_wait_ack_typed(...)`

Эти helper-методы уже учитывают fallback-форматы ack.

## Что запускать для диагностики

```bash
RUST_LOG=debug cargo run --example cws_orders_smoke
```

В debug-логах видно:

- WS события
- CWS ack
- статусы ордеров (create/update/delete)

