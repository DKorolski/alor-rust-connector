use super::order_common::{Instrument, OrderSide, TimeInForce, User};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CreateLimitOrderRequest {
    pub opcode: String,
    pub guid: String,
    pub side: OrderSide,
    pub quantity: i32,
    pub price: f64,
    pub instrument: Instrument,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub user: User,
    #[serde(rename = "timeInForce", skip_serializing_if = "Option::is_none")]
    pub time_in_force: Option<TimeInForce>,
    #[serde(rename = "allowMargin", skip_serializing_if = "Option::is_none")]
    pub allow_margin: Option<bool>,
    #[serde(rename = "icebergFixed", skip_serializing_if = "Option::is_none")]
    pub iceberg_fixed: Option<i32>,
    #[serde(rename = "icebergVariance", skip_serializing_if = "Option::is_none")]
    pub iceberg_variance: Option<i32>,
    #[serde(rename = "checkDuplicates", skip_serializing_if = "Option::is_none")]
    pub check_duplicates: Option<bool>,
}
