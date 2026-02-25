use super::order_common::User;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DeleteOrderRequest {
    pub opcode: String,
    pub guid: String,
    #[serde(rename = "orderId")]
    pub order_id: String,
    pub exchange: String,
    pub user: User,
    #[serde(rename = "checkDuplicates", skip_serializing_if = "Option::is_none")]
    pub check_duplicates: Option<bool>,
}
