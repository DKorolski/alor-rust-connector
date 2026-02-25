use anyhow::Result;
use reqwest::Url;
use serde_json::Value;
use std::str::FromStr;
use tokio_tungstenite::tungstenite::http::Uri;

use crate::helpers::servers::get_server_url;
use crate::structs::ws::WebSocketState;

// #[derive(Clone)]
pub struct ApiClient {
    pub client: reqwest::Client,
    pub api_server: Url,
    pub socket_client: WebSocketState,
}

impl ApiClient {
    pub async fn new(
        demo: bool,
        get_bar_callback: fn(&Value),
    ) -> Result<Self> {
        let ws_server = Uri::from_str(get_server_url("ws_server", demo)?.as_str())?;

        Ok(ApiClient {
            client: reqwest::Client::new(),
            api_server: Url::parse(get_server_url("api_server", demo)?.as_str())?,
            socket_client: WebSocketState::new(ws_server, get_bar_callback).await?,
        })
    }
}

impl ApiClient {
    pub async fn validate_response(
        &self,
        response: reqwest::Response,
    ) -> Result<Value> {
        let response_json: Value =
            serde_json::from_str(&response.error_for_status()?.text().await?)?;

        Ok(response_json)
    }
}
