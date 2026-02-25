# Аудит коннектора `alor-rust-connector` и ТЗ на доработку

## Цель документа

Этот документ фиксирует:

- фактическое состояние локального коннектора по исходному коду;
- критические дефекты и архитектурные риски;
- ТЗ на поэтапную доработку (несколько итераций) с критериями приемки.

Важно: аудит выполнен статически по исходникам. Полный `cargo check` в текущем окружении не выполнен из-за отсутствия доступа к `crates.io`.

Дополнение (после последующих доработок и live smoke test):

- подтвержден рабочий сценарий `create -> update -> delete` через `CWS + OrdersGetAndSubscribeV2`;
- подтверждено, что `update:limit` в проверенном кейсе работает как `cancel old + create new` (новый `orderNumber` + два WS статуса).
- добавлены unit-тесты на helper-парсеры event-driven слоя (`guid`, `order id`, `status`) и они проходят локально;
- добавлены typed event wrappers (`CwsAckEvent`, `WsOrderStatusEvent`, `WsSubscribeAckEvent`) и typed helper для ack подписки;
- добавлены `AlorConfig`, `AlorClientBuilder`, `AlorRust::from_config(...)`, `AlorRust::new_default_callbacks(...)` (старт Итерации 3);
- в `src/` runtime-`unwrap/expect/panic!` в ключевых путях убраны; остатки допускаются только в тестах.

## Краткое резюме состояния

- Библиотека объединяет REST (`ApiClient`), market-data WebSocket (`ws`) и command WebSocket (`cws`) в одном типе `AlorRust`.
- Основной код рабочий по структуре; ранее содержал много `unwrap()/expect()/panic!` в runtime-критичных местах, значимая часть уже устранена.
- CWS API для заявок возвращает GUID запроса; получение фактического идентификатора заявки (`orderNumber` / `data.id`) оформлено через публичные helper-методы (`CWS + OrdersGetAndSubscribeV2`) и подтверждено live.
- Примеры приведены к одному рабочему smoke-сценарию (`examples/cws_orders_smoke.rs`) под текущий API.
- README актуализирован под фактический API (event-driven flow, typed helpers, builder/config, ограничения).

## Критический анализ (findings)

### P0 / P1: Надежность и корректность

1. Массовое использование `unwrap/expect/panic` в сетевом коде и инициализации.
- Риск: падение процесса вместо управляемой ошибки при любой сетевой/JSON/протокольной аномалии.
- Примеры: `src/lib.rs`, `src/structs/auth_client.rs`, `src/structs/ws.rs`, `src/structs/cws.rs`.
- Статус: по текущему `src/` runtime-слою практически закрыто; остатки `unwrap` остались только в тестах/комментариях.

2. `AuthClient::parse_data_from_token()` жестко требует `PORTFOLIO_NUMBER` через `env::var(...).unwrap()`.
- Риск: коннектор падает даже при валидном refresh token, если env-переменная не выставлена.
- Это скрытая обязательная зависимость, не отраженная в публичном API.
- Статус: закрыто (panic-path убран; `PORTFOLIO_NUMBER` optional).

3. Ошибочный путь при неуспешном refresh токена.
- В `get_jwt_token_force()` при HTTP error очищаются поля токена, но затем выполняется `raw_response.unwrap()`.
- Риск: panic вместо `Err(...)`.
- Статус: закрыто (возвращается `Err(...)`, состояние токена очищается корректно).

4. `CWS` create/update/delete методы возвращают только `guid`, а сценарий получения идентификатора заявки через `OrdersGetAndSubscribeV2` не оформлен в публичном API.
- Риск: клиент вынужден вручную собирать связку `CWS ack + WS stream статусов`, часто через внутренние поля.
- Это ломает основной use-case "создать -> получить `orderNumber`/`id` -> обновить/удалить".
- Статус: существенно закрыто (добавлены event-stream API, helper-методы высокого уровня `create/update/delete limit`, typed subscribe ack helper; live flow подтвержден).

5. В `CWS::socket_listener()` сообщения без `requestGuid` фактически теряются (только debug-log).
- Риск: пуш-события/асинхронные события сервера не доходят до пользователя.
- Статус: закрыто (роутятся в `global_callback`, публикуются в event stream).

6. В `CWS::authenticate()` отсутствует timeout ожидания ответа.
- Риск: вечное зависание при потере ответа/разрыве соединения.
- Статус: закрыто (добавлен timeout).

7. `WebSocketState`/`CWS` не имеют стратегии reconnect/resubscribe.
- Риск: после разрыва соединения клиент остается в деградированном состоянии.

### P1 / P2: API/UX и сопровождение

8. `AlorRust::new()` всегда поднимает и REST, и `ws`, и `cws`.
- Риск: нельзя использовать библиотеку только для REST; падение любой подсистемы ломает весь конструктор.
- Статус: частично закрыто (добавлены `AlorConfig`/builder и `from_config`, но REST-only/lazy init еще не реализован).

9. В `AuthClient::parse_data_from_token()` аккаунты накапливаются при повторных refresh (нет очистки `user_data.accounts`).
- Риск: дубли в `accounts()` и потенциально неверный выбор счетов.
- Статус: закрыто (список аккаунтов очищается перед повторным парсингом).

10. `subscribe_bars()` игнорирует результат отправки (`let _result = ...`).
- Риск: метод возвращает `guid`, даже если подписка не была отправлена.
- Статус: закрыто (ошибка отправки возвращается через `Result`).

11. API времени семантически запутан:
- `utc_timestamp_to_msk_datetime()` возвращает `DateTime<Utc>` (по имени ожидается MSK).
- `msk_datetime_to_utc_timestamp()` принимает `DateTime<Utc>` (по имени ожидается московское локальное время).

12. Публичные поля раскрывают внутренности (`pub client`, `pub auth_client`, `pub cws_client`, `CWS.write_stream`, `CWS.request_callbacks`).
- Риск: пользователи обходят инварианты и начинают зависеть от внутренних деталей.

13. `init_logger()` вызывает `env_logger::init()` без защиты от повторного вызова.
- Риск: panic при повторной инициализации логгера.
- Статус: закрыто (`try_init()`).

14. `get_history()` создает много задач без лимита параллелизма и использует `std::sync::mpsc` внутри async-кода.
- Риск: перегрузка при больших диапазонах, сложное поведение и лишняя синхронизация.

15. Отсутствуют тесты (`#[test]` / `#[tokio::test]` не найдены).
- Риск: регрессии при изменении DTO, auth и websocket-логики.
- Статус: частично закрыто (добавлены unit-тесты на helper-парсеры/events/config builder; тесты проходят локально).

### Проблемы примеров и документации

16. `examples/cws_example.rs` содержит хардкод refresh token и портфели.
- Риск: утечка секретов/данных в репозитории и плохой пример для пользователей.
- Статус: закрыто (устаревшие примеры удалены, оставлен один актуальный smoke test).

17. `examples/cws_example_1.rs` обращается к `client.cws_client.read_stream`, которого нет в `CWS`.
- Риск: пример не соответствует текущему API (вероятная ошибка компиляции).
- Статус: закрыто (пример удален).

18. `examples/cws_example_2.rs` подставляет фиктивный `order_number = "10"`.
- Риск: вводит в заблуждение как "рабочий" сценарий create -> update -> delete.
- Статус: закрыто (пример удален).

## ТЗ на доработку (итерациями)

Ниже планируется выполнение в 4 итерации. Каждая итерация должна завершаться релизным состоянием библиотеки (пусть и с известными ограничениями).

## Итерация 1. Стабилизация ошибок и базовая эксплуатационная надежность

Статус: практически закрыта (runtime `unwrap/panic` в `src/` вычищены, auth refresh/path исправлены, `PORTFOLIO_NUMBER` optional, `init_logger` безопасен; основной остаток — единый error type и полировка отдельных API-контрактов).

### Цель

Убрать аварийные падения в основных потоках и сделать поведение предсказуемым через `Result`.

### Объем работ

- Заменить `unwrap/expect/panic!` в публичных и сетевых путях на типизированные ошибки.
- Ввести единый error type (например, `AlorError`) с категориями:
- `Auth`
- `Http`
- `WebSocket`
- `Protocol`
- `Serialization`
- `Config`
- Исправить `AuthClient::get_jwt_token_force()` и `get_jwt_token()` на корректный `Err(...)` при неуспешном refresh.
- Сделать `PORTFOLIO_NUMBER` необязательным:
- либо параметром конструктора;
- либо `Option<String>` в конфиге;
- либо fallback-логикой без env.
- В `subscribe_bars()` возвращать ошибку при невозможности отправки сообщения.
- Сделать `init_logger()` безопасным к повторным вызовам (`try_init`).

### Критерии приемки

- Публичные методы `AlorRust`/`CWS`/`AuthClient` не падают panic при типичных сетевых ошибках.
- Нет `unwrap/expect/panic!` в публичных методах и сетевых happy/error path (допустимы только в тестах/невозможных инвариантах).
- Ошибка отсутствия `PORTFOLIO_NUMBER` возвращается как `Err(Config)` либо не требуется вовсе.
- README обновлен с корректным описанием новых требований к конфигу.

## Итерация 2. Нормальный CWS API для заявок

Статус: в основном закрыта для limit-flow (event streams, timeout, non-requestGuid routing, helper-методы `create/update/delete`, typed events/subscribe ack, live smoke подтверждение). Остатки: более общий typed API (`authorize`/прочие команды), cleanup callback lifecycle/утечки под длительной нагрузкой.

### Цель

Сделать торговые операции удобными и безопасными: пользователь должен получать ответ сервера и фактический идентификатор заявки (`orderNumber`/`id`) штатным API.

### Объем работ

- Добавить публичный API ожидания ответа по `requestGuid`:
- `send_and_wait(...)` с timeout;
- или методы высокого уровня `create_limit_order_and_wait(...)`, etc.
- Добавить публичный API подписки на статусы заявок через `OrdersGetAndSubscribeV2` (WS) и связку с `requestGuid`.
- Зафиксировать источник истины для идентификатора заявки: события статусов заявок (`WS`, поле `data.id`) с сопоставлением по потоку событий/ack.
- Ввести типизированные response structs для create/update/delete/authorize.
- Реализовать timeout и cleanup callback-ов (без утечек в `request_callbacks`).
- Сообщения без `requestGuid` передавать в `global_callback`, а не терять.
- Добавить публичный поток/канал событий CWS (или callback + typed enum событий).
- Зафиксировать контракт: create возвращает `requestGuid`, а helper-метод высокого уровня возвращает `requestGuid` + идентификатор заявки (`orderNumber`/`id`) после подтверждения по WS/CWS.

### Критерии приемки

- Есть рабочий пример create -> получить идентификатор заявки из `OrdersGetAndSubscribeV2` -> update -> delete без доступа к внутренностям `CWS`.
- Нет зависаний без timeout в `authenticate` и ожидании ответов запросов.
- Push-сообщения CWS доходят до пользователя через публичный API.

## Итерация 3. Управление соединениями и API-эргономика

Статус: начата (добавлены `AlorConfig`, `AlorClientBuilder`, `AlorRust::from_config(...)`, `new_default_callbacks(...)`; REST-only/lazy init и reconnect пока не реализованы).

### Цель

Разделить подсистемы, упростить использование и повысить устойчивость к разрывам.

### Объем работ

- Ввести builder/config (`AlorConfig` / `AlorClientBuilder`):
- `demo`
- `refresh_token`
- optional callbacks
- включение/выключение `ws` и `cws`
- timeout-параметры
- Реализовать lazy init или отдельные клиенты:
- `RestClient`
- `MarketDataWsClient`
- `TradingCwsClient`
- Добавить reconnect strategy для `ws`/`cws` (минимум manual reconnect API, желательно auto reconnect).
- Добавить unsubscribe/resubscribe для market data.
- Скрыть лишние `pub` поля, оставить стабильный публичный интерфейс.
- Исправить API времени (`DateTime<FixedOffset>`/`DateTime<Tz>` или честные названия).

### Критерии приемки

- Возможен REST-only клиент без подключения WebSocket.
- Разрыв `ws/cws` не приводит к неуправляемому падению процесса.
- Публичный API не требует работы с `write_stream`/`request_callbacks` напрямую.

## Итерация 4. Качество, тесты, документация и релизная подготовка

Статус: начата частично (README существенно актуализирован, examples очищены до рабочего smoke test, есть базовые unit-тесты; CI/integration tests/релизная полировка не сделаны).

### Цель

Закрепить поведение тестами и сделать библиотеку пригодной для внешнего использования.

### Объем работ

- Добавить unit-тесты:
- auth token parsing
- URL selection (`demo/prod`)
- DTO serialization (snake/camel/Pascal/rename поля)
- account parsing из JWT
- Добавить integration tests (по возможности через mock server/websocket).
- Очистить примеры:
- убрать секреты;
- актуализировать под новый CWS API;
- разделить `demo` и `prod` сценарии.
- Добавить CI (`cargo check`, `cargo test`, `cargo fmt --check`, `cargo clippy`).
- Расширить README:
- quickstart
- ограничения
- API map
- примеры
- troubleshooting

### Критерии приемки

- Есть автоматические тесты на критичные сценарии сериализации и auth.
- Все examples компилируются.
- Нет секретов/портфелей в репозитории.
- README соответствует фактическому API и сценариям.

## Предлагаемый порядок внедрения (практический)

1. Сначала Итерация 1 + минимальная часть Итерации 2 (`authenticate` timeout, non-requestGuid callback routing).
2. Затем полноценный request/response API для CWS (Итерация 2).
3. После этого рефакторинг архитектуры и builder (Итерация 3).
4. В конце тесты/CI/финальная документация (Итерация 4).

## Что считать "нормальным" результатом после доработки

- Коннектор не падает на сетевых ошибках.
- Пользователь получает идентификатор заявки (`orderNumber`/`id`) штатным API через поддержанный сценарий `CWS + OrdersGetAndSubscribeV2`.
- Есть понятные примеры без доступа к private/internal полям.
- README описывает именно то, что реально поддерживается.
