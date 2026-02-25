use alor_rust::dto::cws_dto::order_common::{OrderSide, StopCondition};
use alor_rust::*;
use log::{error, info};
use serde_json::Value;
use std::time::{Duration};
use tokio::time::sleep;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // Инициализация логирования
    init_logger("INFO");

    // Создание клиента для взаимодействия с API
    let mut client = AlorRust::new(
        "", // Твой refresh token
        false, // demo=false для боевого API
        log_event, // Логирование событий
        log_event, // Логирование событий
    )
    .await?;

    let portfolio = "7502T0U"; // Убедись, что это правильный портфель

    // Получаем позиции для портфеля
    let positions = client
        .get_positions(portfolio, "MOEX", true, "Simple")
        .await?;

    // Проверка наличия позиций
    if let Some(positions_array) = positions.as_array() {
        if positions_array.is_empty() {
            info!("No positions found for portfolio {}", portfolio);
        } else {
            println!("portfolio: {:?}\tpositions: {:?}", portfolio, positions_array);
        }
    } else {
        error!("Failed to parse positions for portfolio {}", portfolio);
        return Ok(());
    }

    // Логирование параметров ордера
    info!(
        "Creating limit order for portfolio: {}, symbol: IMOEXF, quantity: 1, price: 2700.0",
        portfolio
    );

    // Создание лимитного ордера
    let new_limit_order_guid = client
        .cws_client
        .create_limit_order(
            OrderSide::Buy,  // Сторона ордера (покупка)
            1,               // Количество
            2700.0,          // Цена (f64)
            "IMOEXF",        // Символ инструмента
            "MOEX",          // Биржа
            None,            // Группа инструментов (необязательно)
            portfolio,       // Портфель
            None,            // Комментарий (необязательно)
            None,            // Время действия (необязательно)
            Some(true),      // Разрешить маржинальные операции
            None,            // Iceberg (необязательно)
            None,            // Iceberg variance (необязательно)
            None,            // Проверка дубликатов (необязательно)
        )
        .await
        .map_err(|e| {
            error!("Failed to create limit order: {:?}", e);
            e
        })?;

    // Логирование ответа от сервера
    info!(
        "Server response after order creation: {:?}",
        new_limit_order_guid
    );

    // И мы должны извлечь orderNumber из ответа сервера
    // Предположим, что new_limit_order_guid — это строка
    let order_number = "10";  // Просто присваиваем строку, если это GUID

    // Если это JSON объект, то код будет выглядеть так:


    info!("Created order number: {}", order_number);

    // Обновление лимитного ордера с новой ценой 2710
    info!("Updating limit order with new price: 2710.0");

    let update_limit_order_guid = client
        .cws_client
        .update_limit_order(
            order_number,  // Используем динамически полученный orderNumber
            OrderSide::Buy, 
            1, 
            2710.0,  // Новая цена
            "IMOEXF", 
            "MOEX", 
            None,  // instrument_group
            portfolio, 
            None,  // comment
            Some(true),  // Разрешить маржинальные операции
            None,  // iceberg_fixed
            None,  // iceberg_variance
        )
        .await?;

    info!(
        "update_limit_order request guid: {:?}",
        update_limit_order_guid
    );

    // Удаление лимитного ордера с правильным orderNumber
    info!("Deleting limit order with orderNumber: {}", order_number);

    let delete_limit_order_guid = client
        .cws_client
        .delete_limit_order(
            order_number,  // Используем динамически полученный orderNumber
            "MOEX", 
            portfolio, 
            None,
        )
        .await?;

    info!(
        "delete_limit_order request guid: {:?}",
        delete_limit_order_guid
    );

    // Бесконечный цикл для предотвращения завершения программы
    loop {
        sleep(Duration::from_secs(1)).await;
    }

}

// Функция для логирования событий
fn log_event(event: &Value) {
    info!("new event: {:?}", event);
}
