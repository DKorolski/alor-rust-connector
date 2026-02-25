use super::order_common::{Instrument, OrderSide, User};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UpdateLimitOrderRequest {
    pub opcode: String,
    pub guid: String,
    #[serde(rename = "orderId")]
    pub order_id: String,
    pub side: OrderSide,
    pub quantity: i32,
    pub price: f64,
    pub instrument: Instrument,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub user: User,
    #[serde(rename = "allowMargin", skip_serializing_if = "Option::is_none")]
    pub allow_margin: Option<bool>,
    #[serde(rename = "icebergFixed", skip_serializing_if = "Option::is_none")]
    pub iceberg_fixed: Option<i32>,
    #[serde(rename = "checkDuplicates", skip_serializing_if = "Option::is_none")]
    pub check_duplicates: Option<bool>,
}
