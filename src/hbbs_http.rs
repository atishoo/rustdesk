use crate::common::PUBLIC_API_SERVER;
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
#[cfg(all(
    feature = "flutter",
    not(any(target_os = "android", target_os = "ios"))
))]
pub(crate) mod address_book;
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
const ACCESS_TOKEN_ACCOUNT: &str = "access_token_account";

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

fn explicit_account_wss(options: &HashMap<String, String>, server: &str) -> Option<String> {
    if server.contains("://") {
        return None;
    }
    let server = crate::client::rendezvous_server_address(server);
    if server.parse::<SocketAddr>().is_ok() || hbb_common::is_ip_str(&server) {
        return None;
    }
    let (host, _) = hbb_common::socket_client::split_host_port(&server)?;
    let port = account_websocket_port(options)?;
    Some(format!("wss://{host}:{port}/ws/id"))
}

fn account_websocket_route(options: &HashMap<String, String>, endpoint: &str) -> Option<String> {
    if endpoint.contains("://") {
        return None;
    }
    if let Ok(address) = endpoint.parse::<SocketAddr>() {
        let port = address.port().checked_add(2)?;
        return Some(format!("ws://{}", SocketAddr::new(address.ip(), port)));
    }
    let (host, _) = hbb_common::socket_client::split_host_port(endpoint)?;
    let scheme = if options
        .get("api-server")
        .is_some_and(|api| api.starts_with("https"))
    {
        "wss"
    } else {
        "ws"
    };
    let port = account_websocket_port(options).map_or_else(String::new, |port| format!(":{port}"));
    Some(format!("{scheme}://{host}{port}/ws/id"))
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

async fn account_options_async() -> HashMap<String, String> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    return hbb_common::config::Config::get_options();
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    crate::ipc::get_options_async().await
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

pub(crate) fn confirmed_account_options() -> Option<[String; 2]> {
    account_options_for_password().ok().flatten()
}

fn account_options_for_password() -> Result<Option<[String; 2]>, ()> {
    let [token, binding] = LocalConfig::get_options([ACCESS_TOKEN, ACCESS_TOKEN_DEPLOYMENT]);
    if LocalConfig::get_option_from_file(ACCESS_TOKEN) != token
        || LocalConfig::get_option_from_file(ACCESS_TOKEN_DEPLOYMENT) != binding
    {
        return Err(());
    }
    match (token.is_empty(), binding.is_empty()) {
        (true, true) => Ok(None),
        (false, false) => Ok(Some([token, binding])),
        _ => Err(()),
    }
}

fn fixed_account_route_matches(
    options: &HashMap<String, String>,
    server: &str,
    endpoint: &str,
) -> bool {
    if !crate::client::valid_uri_rendezvous(server) {
        return false;
    }
    let base = crate::client::rendezvous_server_address(server);
    [
        account_websocket_route(options, &base),
        Some(base),
        explicit_account_wss(options, server),
    ]
    .iter()
    .flatten()
    .any(|route| crate::client::same_rendezvous_endpoint(endpoint, route))
}

pub(crate) fn password_deployment_for_route(
    options: &HashMap<String, String>,
    endpoint: &str,
    key: &str,
) -> Option<String> {
    if hbb_common::config::is_disable_account() {
        return None;
    }
    let (api, server, configured_key, deployment) = account_deployment(options);
    let public_endpoint = crate::client::is_public_rendezvous_endpoint(endpoint);
    let route_matches = if server.is_empty() {
        api == PUBLIC_API_SERVER && public_endpoint
    } else {
        (api != PUBLIC_API_SERVER || public_endpoint)
            && fixed_account_route_matches(options, &server, endpoint)
    };
    (!api.is_empty()
        && (configured_key == key
            || (configured_key.is_empty() && key == hbb_common::config::RS_PUB_KEY))
        && route_matches)
        .then_some(deployment)
}

pub(crate) fn password_account_is_current(owner: &str) -> bool {
    let Some([token, binding]) = confirmed_account_options() else {
        return false;
    };
    let deployment = account_deployment(&hbb_common::config::Config::get_options()).3;
    owner == binding && deployment_digest(&[&deployment, &token]) == binding
}

fn account_id_for_binding(binding: &str) -> Option<u64> {
    let value = LocalConfig::get_option(ACCESS_TOKEN_ACCOUNT);
    if value != LocalConfig::get_option_from_file(ACCESS_TOKEN_ACCOUNT) {
        return None;
    }
    let (stored_binding, id) = value.split_once(':')?;
    (binding == stored_binding).then_some(id)?.parse().ok()
}

fn password_owner(deployment: &str, binding: &str, id: Option<u64>) -> String {
    match id {
        Some(id) => deployment_digest(&[deployment, "user", &id.to_string()]),
        None => deployment_digest(&[deployment, "session", binding]),
    }
}

pub(crate) fn password_owner_for_binding(deployment: Option<&str>, binding: &str) -> String {
    password_owner(
        deployment.unwrap_or_default(),
        binding,
        account_id_for_binding(binding),
    )
}

pub(crate) fn remembered_password_owner(deployment: Option<&str>) -> Result<Option<String>, ()> {
    let Some(deployment) = deployment else {
        return Ok(None);
    };
    let Some([token, binding]) = account_options_for_password()? else {
        return Ok(None);
    };
    if deployment_digest(&[deployment, &token]) != binding {
        return Err(());
    }
    Ok(Some(password_owner_for_binding(Some(deployment), &binding)))
}

pub(crate) fn current_password_account() -> Option<(String, String)> {
    let deployment = account_deployment(&hbb_common::config::Config::get_options()).3;
    let [token, binding] = confirmed_account_options()?;
    if deployment_digest(&[&deployment, &token]) != binding {
        return None;
    }
    let owner = password_owner(&deployment, &binding, account_id_for_binding(&binding));
    Some((binding, owner))
}

#[derive(Default)]
pub(crate) struct PasswordAccountVerification {
    session: Option<(String, Result<u64, ()>)>,
}

impl PasswordAccountVerification {
    async fn user_id(&mut self, api: &str, token: &str, binding: &str) -> Result<u64, ()> {
        if let Some((checked, result)) = &self.session {
            if checked == binding {
                return *result;
            }
        }
        let result = fetch_password_account_id(api, token).await;
        self.session = Some((binding.to_owned(), result));
        result
    }
}

async fn fetch_password_account_id(api: &str, token: &str) -> Result<u64, ()> {
    let client = device_credential::http_client(api).map_err(|_| ())?;
    let response = client
        .get(format!("{api}/api/user/info"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| ())?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(());
    }
    let user = response.json::<Value>().await.map_err(|_| ())?;
    user.get("id")
        .and_then(Value::as_u64)
        .filter(|&id| id != 0)
        .ok_or(())
}

pub(crate) async fn authorize_cached_device_password(
    password_deployment: Option<&str>,
    password_account: Option<&str>,
    remembered_owner: Option<&str>,
    verification: &mut PasswordAccountVerification,
) -> Result<Option<String>, ()> {
    if password_deployment.is_none() && password_account.is_none() && remembered_owner.is_none() {
        return Ok(None);
    }
    let (api, _, _, deployment) = account_deployment(&account_options_async().await);
    let Some([token, binding]) = confirmed_account_options() else {
        return Err(());
    };
    if password_deployment.is_some_and(|owner| owner != deployment)
        || password_account.is_some_and(|owner| owner != binding)
        || deployment_digest(&[&deployment, &token]) != binding
    {
        return Err(());
    }
    let id = verification.user_id(&api, &token, &binding).await?;
    if remembered_owner.is_some_and(|owner| {
        owner != password_owner(&deployment, &binding, Some(id))
            && owner != password_owner(&deployment, &binding, None)
    }) || account_deployment(&account_options_async().await).3 != deployment
        || confirmed_account_options() != Some([token, binding.clone()])
    {
        return Err(());
    }
    Ok(Some(binding))
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

pub(crate) fn store_account_token(token: String, deployment: &str, user_id: Option<u64>) {
    let binding = if token.is_empty() || deployment.is_empty() {
        String::new()
    } else {
        deployment_digest(&[deployment, &token])
    };
    let account = user_id
        .filter(|&id| id != 0 && !binding.is_empty())
        .map(|id| format!("{binding}:{id}"))
        .unwrap_or_default();
    LocalConfig::set_options([
        (ACCESS_TOKEN, token),
        (ACCESS_TOKEN_DEPLOYMENT, binding),
        (ACCESS_TOKEN_ACCOUNT, account),
    ]);
    device_credential::CHANGED.notify_one();
}

pub(crate) async fn account_token_matches(
    token: &str,
    endpoint: &str,
    key: &str,
    strict: bool,
    fixed_native: bool,
) -> bool {
    let read_account = || {
        if fixed_native {
            confirmed_account_options()
        } else {
            Some(LocalConfig::get_options([ACCESS_TOKEN, ACCESS_TOKEN_DEPLOYMENT]))
        }
    };
    let Some([stored_token, binding]) = read_account() else {
        return false;
    };
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
        api == PUBLIC_API_SERVER && is_public_endpoint
    } else if fixed_native && !strict {
        fixed_account_route_matches(&options, &current_server, endpoint)
    } else {
        if !crate::client::valid_uri_rendezvous(&current_server) {
            return false;
        }
        let mut expected = crate::client::rendezvous_transport_endpoint(&current_server);
        if endpoint.starts_with("wss://") {
            if let Some(explicit) = explicit_account_wss(&options, &current_server) {
                expected = explicit;
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
        && (api != PUBLIC_API_SERVER || is_public_endpoint)
        && bound_account_token(stored_token.clone(), binding.clone(), &deployment) == token
        && read_account() == Some([stored_token, binding])
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
