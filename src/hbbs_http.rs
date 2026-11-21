use hbb_common::{
    config::LocalConfig,
    sha2::{Digest, Sha256},
    ResultType,
};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use std::{collections::HashMap, net::SocketAddr};

#[cfg(feature = "flutter")]
pub mod account;
pub mod downloader;
pub(crate) mod device_credential;
mod http_client;
pub mod record_upload;
pub mod sync;
pub use http_client::{
    create_http_client_async, create_http_client_async_with_url_strict,
    create_http_client_with_url, create_http_client_with_url_strict, get_url_for_tls,
};

const ACCESS_TOKEN: &str = "access_token";
const ACCESS_TOKEN_DEPLOYMENT: &str = "access_token_deployment";

fn deployment_digest(parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part.as_bytes());
    }
    hex::encode(digest.finalize())
}

fn account_websocket_port(options: &HashMap<String, String>) -> Option<u16> {
    options
        .get(hbb_common::config::keys::OPTION_WEBSOCKET_PORT)
        .and_then(|value| value.parse::<std::num::NonZeroU16>().ok())
        .map(std::num::NonZeroU16::get)
}

fn account_deployment(options: &HashMap<String, String>) -> (String, String, String, String) {
    let option = |name: &str| options.get(name).map_or("", String::as_str);
    let custom = option("custom-rendezvous-server");
    let api = effective_api_server(options);
    let server = crate::get_custom_rendezvous_server(custom.to_owned());
    let key = crate::common::configured_key(option("key").to_owned());
    let endpoint = if server.is_empty() {
        String::new()
    } else {
        crate::client::rendezvous_server_address(&server)
    };
    let identity = crate::client::rendezvous_endpoint_identity(&endpoint);
    let binding = if !server.contains("://")
        && endpoint.parse::<SocketAddr>().is_err()
        && !hbb_common::is_ip_str(&endpoint)
        && hbb_common::socket_client::split_host_port(&endpoint).is_some()
    {
        // A bare host can route through the configured WSS port. Explicit URLs and IPs do not.
        let default_port = if option("api-server").starts_with("https") {
            443
        } else {
            80
        };
        let port = account_websocket_port(options)
            .map(|port| format!("explicit:{port}"))
            .unwrap_or_else(|| format!("default:{default_port}"));
        deployment_digest(&[&api, &identity, &key, &port])
    } else {
        deployment_digest(&[&api, &identity, &key])
    };
    (api, server, key, binding)
}

fn account_options() -> HashMap<String, String> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    return hbb_common::config::Config::get_options();
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    crate::ipc::get_options()
}

fn effective_api_server(options: &HashMap<String, String>) -> String {
    crate::get_api_server(
        options.get("api-server").cloned().unwrap_or_default(),
        options.get("custom-rendezvous-server").cloned().unwrap_or_default(),
    )
}

pub(crate) fn account_api_server() -> String {
    effective_api_server(&account_options())
}

pub(crate) fn account_login_context() -> (String, String) {
    let (api, _, _, deployment) = account_deployment(&account_options());
    (api, deployment)
}

fn bound_account_token(token: String, binding: String, deployment: &str) -> String {
    if !token.is_empty()
        && !binding.is_empty()
        && binding == deployment_digest(&[deployment, &token])
    {
        token
    } else {
        String::new()
    }
}

pub(crate) fn account_request_context() -> (String, String) {
    let (api, _, _, deployment) = account_deployment(&account_options());
    let [token, binding] = LocalConfig::get_options([ACCESS_TOKEN, ACCESS_TOKEN_DEPLOYMENT]);
    (api, bound_account_token(token, binding, &deployment))
}


#[cfg(feature = "flutter")]
pub(crate) fn account_server_identity_changed(api: String, id: String, key: String) -> bool {
    let mut options = account_options();
    let (_, _, _, previous) = account_deployment(&options);
    {
        use hbb_common::config::{DEFAULT_SETTINGS, OVERWRITE_SETTINGS};
        let overwrite = OVERWRITE_SETTINGS.read().unwrap();
        let defaults = DEFAULT_SETTINGS.read().unwrap();
        for (name, requested) in [
            ("api-server", api),
            ("custom-rendezvous-server", id),
            ("key", key),
        ] {
            let effective = overwrite
                .get(name)
                .cloned()
                .or_else(|| (!requested.is_empty()).then_some(requested))
                .or_else(|| defaults.get(name).cloned());
            if let Some(effective) = effective {
                options.insert(name.to_owned(), effective);
            } else {
                options.remove(name);
            }
        }
    }
    account_deployment(&options).3 != previous
}

pub(crate) fn store_account_token(token: String, deployment: &str) {
    let binding = if token.is_empty() || deployment.is_empty() {
        String::new()
    } else {
        deployment_digest(&[deployment, &token])
    };
    LocalConfig::set_options([(ACCESS_TOKEN, token), (ACCESS_TOKEN_DEPLOYMENT, binding)]);
    device_credential::CHANGED.notify_one();
}

pub(crate) async fn account_token_matches(
    token: &str,
    endpoint: &str,
    key: &str,
    strict: bool,
) -> bool {
    let [stored_token, binding] = LocalConfig::get_options([ACCESS_TOKEN, ACCESS_TOKEN_DEPLOYMENT]);
    if token.is_empty() || stored_token != token {
        return false;
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let options = hbb_common::config::Config::get_options();
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let options = crate::ipc::get_options_async().await;
    let (api, current_server, current_key, deployment) = account_deployment(&options);
    let is_public_endpoint = crate::client::is_public_rendezvous_endpoint(endpoint);
    let matches_endpoint = if current_server.is_empty() {
        api == "https://admin.rustdesk.com" && is_public_endpoint
    } else {
        if !crate::client::valid_uri_rendezvous(&current_server) {
            return false;
        }
        let mut expected = crate::client::rendezvous_transport_endpoint(&current_server);
        if endpoint.starts_with("wss://") && !current_server.contains("://") {
            let current = crate::client::rendezvous_server_address(&current_server);
            if current.parse::<SocketAddr>().is_err() && !hbb_common::is_ip_str(&current) {
                if let Some(((host, _), port)) = hbb_common::socket_client::split_host_port(&current)
                    .zip(account_websocket_port(&options))
                {
                    expected = format!("wss://{host}:{port}/ws/id");
                }
            }
        }
        crate::client::same_rendezvous_endpoint(endpoint, &expected)
    };
    (current_key == key
        || (current_key.is_empty()
            && key == hbb_common::config::RS_PUB_KEY
            && (!strict || is_public_endpoint)))
        && (!strict || key.is_empty() || crate::get_rs_pk(key).is_some())
        && matches_endpoint
        && bound_account_token(stored_token.clone(), binding.clone(), &deployment) == token
        && LocalConfig::get_options([ACCESS_TOKEN, ACCESS_TOKEN_DEPLOYMENT]) == [stored_token, binding]
}

#[derive(Debug)]
pub enum HbbHttpResponse<T> {
    ErrorFormat,
    Error(String),
    DataTypeFormat,
    Data(T),
}

impl<T: DeserializeOwned> HbbHttpResponse<T> {
    pub fn parse(body: &str) -> ResultType<Self> {
        let map = serde_json::from_str::<Map<String, Value>>(body)?;
        if let Some(error) = map.get("error") {
            if let Some(err) = error.as_str() {
                Ok(Self::Error(err.to_owned()))
            } else {
                Ok(Self::ErrorFormat)
            }
        } else {
            match serde_json::from_value(Value::Object(map)) {
                Ok(v) => Ok(Self::Data(v)),
                Err(_) => Ok(Self::DataTypeFormat),
            }
        }
    }
}

#[cfg(test)]
mod account_request_tests {
    use super::{bound_account_token, deployment_digest};

    #[test]
    fn token_is_only_available_for_its_login_deployment() {
        let token = "deployment-a-token".to_owned();
        let deployment_a = deployment_digest(&["https://a.example", "server-a"]);
        let deployment_b = deployment_digest(&["https://b.example", "server-b"]);
        let binding = deployment_digest(&[&deployment_a, &token]);

        assert_eq!(bound_account_token(token.clone(), binding.clone(), &deployment_a), token);
        assert!(bound_account_token(token.clone(), binding.clone(), &deployment_b).is_empty());
        assert!(bound_account_token("other-token".into(), binding, &deployment_a).is_empty());
    }
}
