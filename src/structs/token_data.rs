use serde_json::Value;

#[derive(Clone)]
pub struct TokenData {
    pub raw_response: Option<String>,
    pub refresh_token: String,
    pub token: Option<String>,

    pub token_decoded: Option<Value>,

    pub token_issued: i64,
    pub token_ttl: i8,
}