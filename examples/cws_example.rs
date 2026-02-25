use alor_rust::dto::cws_dto::order_common::{OrderSide, StopCondition};
use alor_rust::*;
use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Utc};
use log::{error, info};
use serde_json::Value;
use std::any::Any;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    init_logger("INFO");

    let mut client = AlorRust::new(
        "ca8bd8a1-aa50-42da-b07b-68b7f3dbe23f",
        true,
        log_event,
        log_event,
    )
    .await?;

    for portfolio in vec![
        "7500297", "75005C1", "7500LMQ", "7500015", "D0547", "D0547", "D32619", "D00015", "G11372",
        "G00015",
    ] {
        let positions = client
            .get_positions(portfolio, "MOEX", true, "Simple")
            .await?;
        println!("portfolio: {:?}\tpositions: {:?}", portfolio, positions);
    }

    let new_market_order_guid = client
        .cws_client
        .create_market_order(
            OrderSide::Buy, // side - обязательный параметр
            300,            // quantity - обязательный параметр
            "SBER",         // symbol - обязательный параметр
            "MOEX",         // exchange - обязательный параметр
            None,           // instrument_group - опциональный параметр
            "D32619",       // portfolio - обязательный параметр
            None,           // comment - опциональный параметр
            None,           // time_in_force - опциональный параметр
            None,           // allow_margin - опциональный параметр
            None,           // check_duplicates - опциональный параметр
        )
        .await?;

    info!(
        "create_market_order request guid: {:?}",
        new_market_order_guid
    );

    let new_limit_order_guid = client
        .cws_client
        .create_limit_order(
            OrderSide::Buy, // side - обязательный параметр
            300,            // quantity - обязательный параметр
            300.0,
            "SBER",   // symbol - обязательный параметр
            "MOEX",   // exchange - обязательный параметр
            None,     // instrument_group - опциональный параметр
            "D32619", // portfolio - обязательный параметр
            None,     // comment - опциональный параметр
            None,     // time_in_force - опциональный параметр
            None,     // allow_margin - опциональный параметр
            None,     // iceberg_fixed - опциональный параметр
            None,     // iceberg_variance - опциональный параметр
            None,     // check_duplicates - опциональный параметр
        )
        .await?;

    info!(
        "create_limit_order request guid: {:?}",
        new_limit_order_guid
    );

    let new_stop_order_guid = client
        .cws_client
        .create_stop_order(
            OrderSide::Buy,
            300,
            StopCondition::Less,
            300.0,
            "SBER",
            "MOEX",
            None,
            "D32619",
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await?;

    info!("create_stop_order request guid: {:?}", new_stop_order_guid);

    let new_stop_limit_order_guid = client
        .cws_client
        .create_stop_limit_order(
            OrderSide::Buy,
            300,
            StopCondition::Less,
            300.0,
            301.0,
            "SBER",
            "MOEX",
            None,
            "D32619",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await?;

    info!(
        "create_stop_limit_order request guid: {:?}",
        new_stop_limit_order_guid
    );

    let update_market_order_guid = client
        .cws_client
        .update_market_order(
            "10",
            OrderSide::Buy, // side - обязательный параметр
            300,            // quantity - обязательный параметр
            "SBER",         // symbol - обязательный параметр
            "MOEX",         // exchange - обязательный параметр
            None,           // instrument_group - опциональный параметр
            "D32619",       // portfolio - обязательный параметр
            None,           // comment - опциональный параметр
            None,           // time_in_force - опциональный параметр
            None,           // allow_margin - опциональный параметр
            None,           // check_duplicates - опциональный параметр
        )
        .await?;

    info!(
        "update_market_order request guid: {:?}",
        update_market_order_guid
    );

    let update_limit_order_guid = client
        .cws_client
        .update_limit_order(
            "10",
            OrderSide::Buy, // side - обязательный параметр
            300,            // quantity - обязательный параметр
            300.0,
            "SBER",   // symbol - обязательный параметр
            "MOEX",   // exchange - обязательный параметр
            None,     // instrument_group - опциональный параметр
            "D32619", // portfolio - обязательный параметр
            None,     // comment - опциональный параметр
            None,     // allow_margin - опциональный параметр
            None,     // iceberg_fixed - опциональный параметр
            None,     // check_duplicates - опциональный параметр
        )
        .await?;

    info!(
        "update_limit_order request guid: {:?}",
        update_limit_order_guid
    );

    let update_stop_order_guid = client
        .cws_client
        .update_stop_order(
            "57290",
            OrderSide::Buy,
            350,
            StopCondition::Less,
            350.0,
            "SBER",
            "MOEX",
            None,
            "D32619",
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await?;

    info!(
        "update_stop_order request guid: {:?}",
        update_stop_order_guid
    );

    let update_stop_limit_order_guid = client
        .cws_client
        .update_stop_limit_order(
            "57292",
            OrderSide::Buy,
            300,
            StopCondition::Less,
            300.0,
            301.0,
            "SBER",
            "MOEX",
            None,
            "D32619",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await?;

    info!(
        "update_stop_limit_order request guid: {:?}",
        update_stop_limit_order_guid
    );

    let delete_market_order_guid = client
        .cws_client
        .delete_market_order(
            "10", "MOEX",   // exchange - обязательный параметр
            "D32619", // portfolio - обязательный параметр
            None,
        )
        .await?;

    info!(
        "delete_market_order request guid: {:?}",
        delete_market_order_guid
    );

    let delete_limit_order_guid = client
        .cws_client
        .delete_limit_order(
            "10", "MOEX",   // exchange - обязательный параметр
            "D32619", // portfolio - обязательный параметр
            None,
        )
        .await?;

    info!(
        "delete_limit_order request guid: {:?}",
        delete_limit_order_guid
    );

    let delete_stop_order_guid = client
        .cws_client
        .delete_stop_order(
            "57294", "MOEX",   // exchange - обязательный параметр
            "D32619", // portfolio - обязательный параметр
            None,
        )
        .await?;

    info!(
        "delete_stop_order request guid: {:?}",
        delete_stop_order_guid
    );

    let delete_stop_limit_order_guid = client
        .cws_client
        .delete_stop_limit_order(
            "57296", "MOEX",   // exchange - обязательный параметр
            "D32619", // portfolio - обязательный параметр
            None,
        )
        .await?;

    info!(
        "delete_stop_limit_order request guid: {:?}",
        delete_stop_limit_order_guid
    );

    // бесконечны цикл чтобы приложение не завершилось само по себе
    loop {
        sleep(Duration::from_secs(1)).await;
    }
    Ok(())
}

fn log_event(event: &Value) {
    info!("new event: {:?}", event)
}
