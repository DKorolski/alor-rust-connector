use serde_json::Value;
use std::fmt;

#[derive(Debug, Clone)]
pub struct CwsAckEvent {
    pub raw: Value,
    pub http_code: Option<u64>,
    pub message: Option<String>,
    pub request_guid: Option<String>,
    pub order_number: Option<String>,
}

impl fmt::Display for CwsAckEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.raw)
    }
}

#[derive(Debug, Clone)]
pub struct WsOrderStatusEvent {
    pub raw: Value,
    pub guid: Option<String>,
    pub order_id: Option<String>,
    pub status: Option<String>,
    pub portfolio: Option<String>,
    pub symbol: Option<String>,
}

impl fmt::Display for WsOrderStatusEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.raw)
    }
}

