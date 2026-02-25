use std::collections::HashMap;

use anyhow::{Error, Result};
use log::debug;

pub fn get_server_url(key: &str, demo: bool) -> Result<String> {
    let urls_map = HashMap::from([
        // dev urls
        ("oauth_server-dev", "https://oauthdev.alor.ru"),
        ("api_server-dev", "https://apidev.alor.ru"),
        ("cws_server-dev", "wss://apidev.alor.ru/cws"),
        ("ws_server-dev", "wss://apidev.alor.ru/ws"),
        // prod urls
        ("oauth_server", "https://oauth.alor.ru"),
        ("api_server", "https://api.alor.ru"),
        ("cws_server", "wss://api.alor.ru/cws"),
        ("ws_server", "wss://api.alor.ru/ws"),
    ]);
    debug!("requested api key: {:?}", key);
    let request_key = if demo { format!("{}-dev", key) } else { key.to_string() };

    if let Some(target_url) = urls_map.get(request_key.as_str()) {
        Ok(target_url.to_string())
    } else {
        Err(Error::msg(format!(
            "incorrect key (\"{:?}\") for server url",
            request_key
        )))
    }
}
