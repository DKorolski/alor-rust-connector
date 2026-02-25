use super::order_common::{Instrument, OrderSide, StopCondition, User};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UpdateStopOrderRequest {
    pub opcode: String,
    pub guid: String,
    #[serde(rename = "orderId")]
    pub order_id: String,
    pub side: OrderSide,
    pub quantity: i32,
    pub condition: StopCondition,
    #[serde(rename = "triggerPrice")]
    pub trigger_price: f64,
    #[serde(rename = "stopEndUnixTime", skip_serializing_if = "Option::is_none")]
    pub stop_end_unix_time: Option<i64>,
    pub instrument: Instrument,
    pub user: User,
    #[serde(rename = "allowMargin", skip_serializing_if = "Option::is_none")]
    pub allow_margin: Option<bool>,
    #[serde(rename = "checkDuplicates", skip_serializing_if = "Option::is_none")]
    pub check_duplicates: Option<bool>,
    #[serde(rename = "protectingSeconds", skip_serializing_if = "Option::is_none")]
    pub protecting_seconds: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activate: Option<bool>,
}
