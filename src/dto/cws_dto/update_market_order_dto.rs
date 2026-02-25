use super::order_common::{Instrument, OrderSide, TimeInForce, User};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UpdateMarketOrderRequest {
    pub opcode: String,
    pub guid: String,
    #[serde(rename = "orderId")]
    pub order_id: String,
    pub side: OrderSide,
    pub quantity: i32,
    pub instrument: Instrument,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub user: User,
    #[serde(rename = "timeInForce", skip_serializing_if = "Option::is_none")]
    pub time_in_force: Option<TimeInForce>,
    #[serde(rename = "allowMargin", skip_serializing_if = "Option::is_none")]
    pub allow_margin: Option<bool>,
    #[serde(rename = "checkDuplicates", skip_serializing_if = "Option::is_none")]
    pub check_duplicates: Option<bool>,
}
