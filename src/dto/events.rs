use serde::Deserialize;
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

impl CwsAckEvent {
    pub fn from_raw(raw: Value) -> Self {
        let parsed = serde_json::from_value::<WireCwsAck>(raw.clone()).ok();
        Self {
            http_code: parsed.as_ref().and_then(|p| p.http_code),
            message: parsed.as_ref().and_then(|p| p.message.clone()),
            request_guid: parsed.as_ref().and_then(|p| p.request_guid.clone()),
            order_number: parsed
                .as_ref()
                .and_then(|p| p.order_number.as_ref())
                .and_then(value_to_string),
            raw,
        }
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

#[derive(Debug, Clone)]
pub struct WsSubscribeAckEvent {
    pub raw: Value,
    pub guid: Option<String>,
    pub request_guid: Option<String>,
    pub http_code: Option<u64>,
    pub message: Option<String>,
    pub data_guid: Option<String>,
}

impl fmt::Display for WsSubscribeAckEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.raw)
    }
}

impl WsOrderStatusEvent {
    pub fn from_raw(raw: Value) -> Self {
        let parsed = serde_json::from_value::<WireWsOrderStatusEnvelope>(raw.clone()).ok();
        let data = parsed.as_ref().and_then(|p| p.data.as_ref());
        Self {
            guid: parsed
                .as_ref()
                .and_then(|p| p.guid.clone().or_else(|| p.request_guid.clone())),
            order_id: data.and_then(|d| d.id.as_ref()).and_then(value_to_string),
            status: data
                .and_then(|d| d.status.as_ref())
                .map(|s| s.to_lowercase()),
            portfolio: data.and_then(|d| d.portfolio.clone()),
            symbol: data.and_then(|d| d.symbol.clone()),
            raw,
        }
    }
}

impl WsSubscribeAckEvent {
    pub fn from_raw(raw: Value) -> Self {
        let parsed = serde_json::from_value::<WireWsSubscribeAck>(raw.clone()).ok();
        let data_guid = raw
            .get("data")
            .and_then(|d| d.get("guid"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        Self {
            guid: parsed.as_ref().and_then(|p| p.guid.clone()),
            request_guid: parsed.as_ref().and_then(|p| p.request_guid.clone()),
            http_code: parsed.as_ref().and_then(|p| p.http_code),
            message: parsed.as_ref().and_then(|p| p.message.clone()),
            data_guid,
            raw,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct WireCwsAck {
    #[serde(rename = "httpCode")]
    http_code: Option<u64>,
    message: Option<String>,
    #[serde(rename = "requestGuid")]
    request_guid: Option<String>,
    #[serde(rename = "orderNumber")]
    order_number: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct WireWsOrderStatusEnvelope {
    guid: Option<String>,
    #[serde(rename = "requestGuid")]
    request_guid: Option<String>,
    data: Option<WireWsOrderStatusData>,
}

#[derive(Debug, Clone, Deserialize)]
struct WireWsSubscribeAck {
    guid: Option<String>,
    #[serde(rename = "requestGuid")]
    request_guid: Option<String>,
    #[serde(rename = "httpCode")]
    http_code: Option<u64>,
    message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WireWsOrderStatusData {
    id: Option<Value>,
    status: Option<String>,
    portfolio: Option<String>,
    symbol: Option<String>,
}

fn value_to_string(v: &Value) -> Option<String> {
    if v.is_string() {
        v.as_str().map(|s| s.to_string())
    } else if v.is_number() {
        Some(v.to_string())
    } else {
        None
    }
}
