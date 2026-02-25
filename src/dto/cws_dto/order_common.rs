use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Instrument {
    pub symbol: String,
    pub exchange: String,
    #[serde(rename = "instrumentGroup", skip_serializing_if = "Option::is_none")]
    pub instrument_group: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct User {
    pub portfolio: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "lowercase")]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "PascalCase")]
pub enum TimeInForce {
    OneDay,
    ImmediateOrCancel,
    FillOrKill,
    AtTheClose,
    GoodTillCancelled,
    BookOrCancel,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "PascalCase")]
pub enum StopCondition {
    More,
    Less,
    MoreOrEqual,
    LessOrEqual,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CreateOrderResponse {
    #[serde(rename = "requestGuid")]
    pub request_guid: String,
    #[serde(rename = "httpCode")]
    pub http_code: i32,
    pub message: String,
    #[serde(rename = "orderNumber", skip_serializing_if = "Option::is_none")]
    pub order_number: Option<String>,
}
