use hbb_common::{
    anyhow::anyhow,
    bail,
    config::{self, Config, LocalConfig, PeerConfig},
    password_security::symmetric_crypt,
    sodiumoxide::base64,
    tokio::{self, sync::Notify},
    ResultType,
};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    collections::HashMap,
    fs,
    io::ErrorKind,
    sync::{Mutex, Once},
    time::Duration,
};

const INTERVAL: Duration = Duration::from_secs(3);
const PERSONAL: &str = "My address book";
const LEGACY: &str = "Legacy address book";
const COMMAND: &str = "address-book-sync";
const FORGET_COMMAND: &str = "address-book-forget-password";
type Recent = HashMap<String, String>;

static SOURCE_CHANGED: Notify = Notify::const_new();
static HOST_CHANGED: Notify = Notify::const_new();
static SOURCE_WORK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static HOST_WORK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static SOURCE: Mutex<Source> = Mutex::new(Source {
    binding: String::new(),
    fingerprint: String::new(),
    book: None,
    full: false,
});
static HOST: Mutex<Host> = Mutex::new(Host {
    loaded: false,
    stored: Vec::new(),
    revision: 0,
});

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Book {
    guid: String,
    personal: bool,
    legacy: bool,
    enabled: bool,
}

#[derive(Clone)]
struct Source {
    binding: String,
    fingerprint: String,
    book: Option<Book>,
    full: bool,
}

#[derive(Serialize, Deserialize)]
struct Payload {
    binding: String,
    source: String,
    book: Option<Book>,
    sync_enabled: bool,
    peers: Vec<Recent>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Stored {
    binding: String,
    source: String,
    book: Book,
    pending: HashMap<String, Recent>,
}

struct Host {
    loaded: bool,
    stored: Vec<Stored>,
    revision: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SyncFile {
    ciphertext: String,
}

#[derive(Clone, PartialEq, Eq)]
struct Context {
    api: String,
    token: String,
    binding: String,
    sync_enabled: bool,
}

pub(crate) fn configure(json: &str) {
    if !crate::flutter::get_global_event_channels()
        .iter()
        .any(|app| app == crate::flutter::APP_TYPE_MAIN)
    {
        return;
    }
    let Ok(cache) = serde_json::from_str::<Value>(json) else {
        return;
    };
    let [token, binding] =
        LocalConfig::get_options([super::ACCESS_TOKEN, super::ACCESS_TOKEN_DEPLOYMENT]);
    if token.is_empty()
        || binding.is_empty()
        || cache["access_token"].as_str() != Some(token.as_str())
    {
        return;
    }
    let name = cache["current_ab_name"].as_str().unwrap_or_default();
    let entry = cache["ab_entries"].as_array().and_then(|entries| {
        entries
            .iter()
            .find(|entry| entry["name"].as_str() == Some(name))
    });
    if cache["sync_enabled"].as_bool() != Some(false) && (name.is_empty() || entry.is_none()) {
        return;
    }
    let guid = entry
        .and_then(|entry| entry["guid"].as_str())
        .unwrap_or_default();
    let book = Book {
        guid: guid.to_owned(),
        personal: name == PERSONAL || name == LEGACY,
        legacy: name == LEGACY,
        enabled: cache["sync_enabled"].as_bool() == Some(true)
            && (name == LEGACY || !guid.is_empty()),
    };
    let fingerprint = super::deployment_digest(&[&entry.unwrap_or(&Value::Null).to_string()]);
    let mut source = SOURCE.lock().unwrap();
    if source.binding != binding
        || source.book.as_ref() != Some(&book)
        || source.fingerprint != fingerprint
    {
        source.binding = binding;
        source.fingerprint = fingerprint;
        source.book = Some(book);
        source.full = true;
        SOURCE_CHANGED.notify_one();
    }
}

pub(crate) async fn collect() {
    let mut interval = tokio::time::interval(INTERVAL);
    let mut revision = None;
    let mut previous_binding = String::new();
    let mut connection = None;
    loop {
        tokio::select! { _ = interval.tick() => {}, _ = SOURCE_CHANGED.notified() => {} }
        let Some([token, binding]) = super::confirmed_account_options() else {
            continue;
        };
        let options = async {
            if connection.is_none() {
                connection = Some(crate::ipc::connect(1000, "").await?);
            }
            crate::ipc::get_options_from_connection(connection.as_mut().unwrap(), 1000).await
        }
        .await;
        let options = match options {
            Ok(options) => options,
            Err(error) => {
                connection = None;
                hbb_common::throttled_log!(
                    crate::common::API_LOG_INTERVAL,
                    warn,
                    "Address book synchronization configuration could not be read: {error}"
                );
                continue;
            }
        };
        let deployment = super::account_deployment(&options).3;
        if super::bound_account_token(token, binding.clone(), &deployment).is_empty() {
            continue;
        }
        if previous_binding != binding {
            previous_binding = binding.clone();
            revision = None;
        }
        let _work = SOURCE_WORK.lock().await;
        let source = SOURCE.lock().unwrap().clone();
        let book = (source.binding == binding)
            .then_some(source.book.clone())
            .flatten();
        let full = source.binding == binding && source.full;
        let ids: Vec<_> = config::NEW_STORED_PEER_CONFIG
            .lock()
            .unwrap()
            .drain()
            .collect();
        if book.is_none() && ids.is_empty() {
            continue;
        }
        let peers = if full || !ids.is_empty() {
            let filters = (!full).then_some(ids.clone());
            let owner =
                super::password_owner(&deployment, &binding, super::account_id_for_binding(&binding));
            match tokio::task::spawn_blocking(move || {
                PeerConfig::peers(filters)
                    .into_iter()
                    .map(|(id, _, config)| {
                        crate::ui_interface::peer_to_map(id, config, Some(&owner))
                            .into_iter()
                            .map(|(key, value)| (key.to_owned(), value))
                            .collect()
                    })
                    .collect()
            })
            .await
            {
                Ok(peers) => peers,
                Err(error) => {
                    config::NEW_STORED_PEER_CONFIG.lock().unwrap().extend(ids);
                    hbb_common::throttled_log!(
                        crate::common::API_LOG_INTERVAL,
                        warn,
                        "Recent connections could not be collected: {error}"
                    );
                    continue;
                }
            }
        } else {
            Vec::new()
        };
        let payload = Payload {
            binding,
            source: Config::path("").to_string_lossy().into_owned(),
            book,
            sync_enabled: LocalConfig::get_option(
                base::config::keys::OPTION_SYNC_AB_WITH_RECENT_SESSIONS,
            ) == "Y",
            peers,
        };
        match submit(&mut connection, &payload).await {
            Ok(next_revision) => {
                if full {
                    let mut current = SOURCE.lock().unwrap();
                    if current.binding == source.binding
                        && current.book == source.book
                        && current.fingerprint == source.fingerprint
                    {
                        current.full = false;
                    }
                }
                if revision.is_some_and(|previous| previous != next_revision)
                    && crate::flutter::get_global_event_channels()
                        .iter()
                        .any(|app| app == crate::flutter::APP_TYPE_MAIN)
                {
                    let _ = crate::flutter::push_global_event(
                        crate::flutter::APP_TYPE_MAIN,
                        json!({"name": "address_book_sync"}).to_string(),
                    );
                }
                revision = Some(next_revision);
            }
            Err(error) => {
                config::NEW_STORED_PEER_CONFIG.lock().unwrap().extend(ids);
                hbb_common::throttled_log!(
                    crate::common::API_LOG_INTERVAL,
                    warn,
                    "Recent connections could not be queued for synchronization: {error}"
                );
            }
        }
    }
}

async fn submit(
    connection: &mut Option<crate::ipc::ConnectionTmpl<parity_tokio_ipc::ConnectionClient>>,
    payload: &Payload,
) -> ResultType<u64> {
    let value = serde_json::to_string(payload)?;
    let result = async {
        let connection = connection
            .as_mut()
            .ok_or_else(|| anyhow!("Address book synchronization connection is unavailable"))?;
        connection
            .send(&crate::ipc::Data::Config((COMMAND.to_owned(), Some(value))))
            .await?;
        match connection.next_timeout(1000).await? {
            Some(crate::ipc::Data::Config((name, result))) if name == COMMAND => Ok(result),
            _ => bail!("Invalid address book synchronization response"),
        }
    }
    .await;
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            *connection = None;
            return Err(error);
        }
    };
    let Some(result) = result else {
        bail!("Background address book synchronization was not acknowledged");
    };
    match serde_json::from_str::<Value>(&result)
        .ok()
        .and_then(|response| response["revision"].as_u64())
    {
        Some(revision) => Ok(revision),
        None => {
            *connection = None;
            bail!("Invalid address book synchronization acknowledgment")
        }
    }
}

pub(crate) async fn forget_password(id: String) -> ResultType<()> {
    let account = LocalConfig::get_options([super::ACCESS_TOKEN, super::ACCESS_TOKEN_DEPLOYMENT]);
    let _work = SOURCE_WORK.lock().await;
    if LocalConfig::get_options([super::ACCESS_TOKEN, super::ACCESS_TOKEN_DEPLOYMENT]) != account {
        bail!("Address book synchronization account changed");
    }
    let [token, binding] = &account;
    if !token.is_empty() && !binding.is_empty() {
        let mut connection = crate::ipc::connect(1000, "").await?;
        connection
            .send(&crate::ipc::Data::Config((
                FORGET_COMMAND.to_owned(),
                Some(serde_json::to_string(&(binding, &id))?),
            )))
            .await?;
        match connection.next_timeout(10_000).await? {
            Some(crate::ipc::Data::Config((name, result)))
                if name == FORGET_COMMAND && result.as_deref() == Some("Y") => {}
            _ => bail!("Background password synchronization could not be cancelled"),
        }
    }
    if LocalConfig::get_options([super::ACCESS_TOKEN, super::ACCESS_TOKEN_DEPLOYMENT]) != account {
        bail!("Address book synchronization account changed");
    }
    crate::ui_interface::forget_password(id)
}

pub(crate) async fn forget_password_request(value: &str) -> Option<String> {
    let (binding, id): (String, String) = serde_json::from_str(value).ok()?;
    let _work = HOST_WORK.lock().await;
    if id.is_empty() || context()?.binding != binding {
        return None;
    }
    let result = (|| -> ResultType<String> {
        let mut host = HOST.lock().unwrap();
        load(&mut host)?;
        let mut jobs = host.stored.clone();
        for job in jobs
            .iter_mut()
            .filter(|job| job.binding == binding && job.book.personal)
        {
            if let Some(peer) = job.pending.get_mut(&id) {
                peer.remove("hash");
            }
        }
        if jobs != host.stored {
            save(&jobs)?;
            host.stored = jobs;
            HOST_CHANGED.notify_one();
        }
        Ok("Y".to_owned())
    })();
    match result {
        Ok(response) => Some(response),
        Err(error) => {
            hbb_common::throttled_log!(
                crate::common::API_LOG_INTERVAL,
                warn,
                "Pending password synchronization could not be cancelled: {error}"
            );
            None
        }
    }
}

fn context() -> Option<Context> {
    let (api, token, local_config) = super::device_credential::host_account()?;
    let saved: hbb_common::toml::Value =
        hbb_common::toml::from_str(&fs::read_to_string(local_config).ok()?).ok()?;
    let key = base::config::keys::OPTION_SYNC_AB_WITH_RECENT_SESSIONS;
    let sync_enabled = config::OVERWRITE_LOCAL_SETTINGS
        .read()
        .unwrap()
        .get(key)
        .map(String::as_str)
        .or(saved
            .get("options")
            .and_then(|options| options.get(key))
            .and_then(|value| value.as_str()))
        .or(config::DEFAULT_LOCAL_SETTINGS
            .read()
            .unwrap()
            .get(key)
            .map(String::as_str))
        == Some("Y");
    let deployment = super::account_deployment(&Config::get_options()).3;
    Some(Context {
        api,
        binding: super::deployment_digest(&[&deployment, &token]),
        token,
        sync_enabled,
    })
}

fn save(stored: &[Stored]) -> ResultType<()> {
    let path = Config::file().with_extension("address-book-sync");
    if stored.is_empty() {
        return match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        };
    }
    let encrypted = symmetric_crypt(&serde_json::to_vec(stored)?, true)
        .map_err(|_| anyhow!("Could not encrypt pending address book synchronization"))?;
    config::store_path(
        path,
        SyncFile {
            ciphertext: base64::encode(&encrypted, base64::Variant::Original),
        },
    )
}

pub(crate) fn clear() -> ResultType<()> {
    let mut host = HOST.lock().unwrap();
    save(&[])?;
    host.loaded = true;
    host.stored.clear();
    Ok(())
}

fn load(host: &mut Host) -> ResultType<()> {
    if host.loaded {
        return Ok(());
    }
    let value = match fs::read_to_string(Config::file().with_extension("address-book-sync")) {
        Ok(value) => value,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            host.loaded = true;
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    let file: SyncFile = hbb_common::toml::from_str(&value)?;
    if file.ciphertext.is_empty() {
        host.loaded = true;
        return Ok(());
    }
    let encrypted = base64::decode(&file.ciphertext, base64::Variant::Original)
        .map_err(|_| anyhow!("Invalid encrypted address book synchronization"))?;
    let plaintext = symmetric_crypt(&encrypted, false)
        .map_err(|_| anyhow!("Could not decrypt pending address book synchronization"))?;
    host.stored = serde_json::from_slice(&plaintext)?;
    host.loaded = true;
    Ok(())
}

pub(crate) fn ipc_request(value: &str) -> Option<String> {
    let payload: Payload = serde_json::from_str(value).ok()?;
    let context = context()?;
    if payload.binding != context.binding || payload.source.is_empty() {
        return None;
    }
    let result = (|| -> ResultType<String> {
        let mut host = HOST.lock().unwrap();
        load(&mut host)?;
        let mut jobs = host.stored.clone();
        jobs.retain(|job| job.binding == payload.binding);
        let configured = payload.book.is_some();
        let index = match payload.book {
            Some(mut book) => {
                book.enabled &= payload.sync_enabled && context.sync_enabled;
                let index = jobs.iter().position(|current| {
                    current.source == payload.source
                        && current.book.guid == book.guid
                        && current.book.personal == book.personal
                        && current.book.legacy == book.legacy
                });
                if let Some(index) = index {
                    let last = jobs
                        .iter()
                        .rposition(|current| current.source == payload.source)
                        .unwrap_or(index);
                    if index != last {
                        let stored = jobs.remove(index);
                        jobs.insert(last, stored);
                    }
                    jobs[last].book = book;
                    last
                } else {
                    jobs.push(Stored {
                        binding: payload.binding.clone(),
                        source: payload.source.clone(),
                        book,
                        pending: HashMap::new(),
                    });
                    jobs.len() - 1
                }
            }
            None => match jobs
                .iter()
                .rposition(|current| current.source == payload.source)
            {
                Some(index) => index,
                None if jobs.is_empty() => {
                    return Ok(json!({"revision": host.revision}).to_string())
                }
                None => {
                    bail!("Address book synchronization has not been configured for this source")
                }
            },
        };
        if !context.sync_enabled || (configured && !payload.sync_enabled) {
            for job in jobs
                .iter_mut()
                .filter(|job| !context.sync_enabled || job.source == payload.source)
            {
                job.book.enabled = false;
                job.pending.clear();
            }
        }
        let stored = &mut jobs[index];
        if stored.book.enabled {
            for peer in payload.peers {
                let Some(id) = peer.get("id").filter(|id| !id.is_empty()) else {
                    continue;
                };
                stored.pending.insert(id.clone(), peer);
            }
        }
        if jobs != host.stored {
            save(&jobs)?;
            host.stored = jobs;
            HOST_CHANGED.notify_one();
        }
        Ok(json!({"revision": host.revision}).to_string())
    })();
    match result {
        Ok(response) => Some(response),
        Err(error) => {
            hbb_common::throttled_log!(
                crate::common::API_LOG_INTERVAL,
                warn,
                "Address book synchronization could not be queued: {error}"
            );
            None
        }
    }
}

fn same_target(stored: &Stored, job: &Stored) -> bool {
    stored.binding == job.binding && stored.source == job.source && stored.book == job.book
}

fn still_current(account: &Context, job: &Stored) -> bool {
    account.sync_enabled
        && context().as_ref() == Some(account)
        && HOST.lock().unwrap().stored.iter().any(|stored| {
            same_target(stored, job)
                && job
                    .pending
                    .iter()
                    .all(|(id, peer)| stored.pending.get(id) == Some(peer))
        })
}

async fn request(
    account: &Context,
    job: &Stored,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> ResultType<Value> {
    let _work = if job.book.legacy {
        None
    } else {
        Some(HOST_WORK.lock().await)
    };
    if !still_current(account, job) {
        bail!("Address book synchronization target changed");
    }
    let mut request = super::device_credential::http_client(&account.api)?
        .request(
            method,
            format!("{}{path}", account.api.trim_end_matches('/')),
        )
        .bearer_auth(&account.token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await?;
    let status = response.status();
    let body = response.text().await?;
    if !still_current(account, job) {
        bail!("Address book synchronization target changed");
    }
    if !status.is_success() {
        bail!("Address book synchronization returned HTTP {status}");
    }
    let value: Value = if body.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&body)?
    };
    if value.get("error").is_some_and(|error| !error.is_null()) {
        bail!("Address book synchronization was rejected by the API");
    }
    Ok(value)
}

fn complete(account: &Context, job: &Stored, ids: &[String], changed: bool) -> ResultType<()> {
    if context().as_ref() != Some(account) {
        bail!("Address book synchronization account changed");
    }
    let mut host = HOST.lock().unwrap();
    let mut jobs = host.stored.clone();
    let stored = jobs
        .iter_mut()
        .find(|current| same_target(current, job))
        .ok_or_else(|| anyhow!("Address book synchronization target changed"))?;
    for id in ids {
        if stored.pending.get(id) == job.pending.get(id) {
            stored.pending.remove(id);
        }
    }
    save(&jobs)?;
    host.stored = jobs;
    if changed {
        host.revision = host.revision.wrapping_add(1);
    }
    Ok(())
}

fn patch(peer: &Value, recent: &Recent, personal: bool) -> Map<String, Value> {
    let mut patch = Map::new();
    for field in ["username", "hostname", "platform", "hash"] {
        if (field == "hash" && !personal)
            || (field != "hash" && peer["sameServer"].as_bool() == Some(true))
        {
            continue;
        }
        if let Some(value) = recent.get(field).filter(|value| !value.is_empty()) {
            if peer[field].as_str().unwrap_or_default() != value {
                patch.insert(field.to_owned(), json!(value));
            }
        }
    }
    patch
}

async fn synchronize(account: &Context, job: &Stored) -> ResultType<()> {
    let mut job = job.clone();
    let guid: String = url::form_urlencoded::byte_serialize(job.book.guid.as_bytes()).collect();
    let response = request(
        account,
        &job,
        Method::POST,
        &format!("/api/ab/peers?ab={guid}"),
        None,
    )
    .await?;
    let peers = response["data"]
        .as_array()
        .ok_or_else(|| anyhow!("Invalid address book peers response"))?;
    let mut unchanged = Vec::new();
    let mut updates = Vec::new();
    for (id, recent) in &job.pending {
        let peer = peers.iter().find(|peer| peer["id"].as_str() == Some(id));
        let mut values = peer
            .map(|peer| patch(peer, recent, job.book.personal))
            .unwrap_or_default();
        if values.is_empty() {
            unchanged.push(id.clone());
        } else {
            values.insert("id".to_owned(), json!(id));
            updates.push((id.clone(), values));
        }
    }
    if !unchanged.is_empty() {
        complete(account, &job, &unchanged, false)?;
        for id in unchanged {
            job.pending.remove(&id);
        }
    }
    for (id, values) in updates {
        request(
            account,
            &job,
            Method::PUT,
            &format!("/api/ab/peer/update/{guid}"),
            Some(Value::Object(values)),
        )
        .await?;
        complete(account, &job, &[id.clone()], true)?;
        job.pending.remove(&id);
    }
    Ok(())
}

async fn synchronize_legacy(account: &Context, job: &Stored) -> ResultType<()> {
    // The legacy API replaces the whole book, including hashes outside this job.
    let _work = HOST_WORK.lock().await;
    let response = request(account, job, Method::GET, "/api/ab", None).await?;
    let limit = response["licensed_devices"].as_u64().unwrap_or_default() as usize;
    let mut data: Value = if response.is_null() {
        json!({"peers": [], "tags": [], "tag_colors": "{}"})
    } else {
        serde_json::from_str(
            response["data"]
                .as_str()
                .ok_or_else(|| anyhow!("Invalid legacy address book response"))?,
        )?
    };
    let peers = data["peers"]
        .as_array_mut()
        .ok_or_else(|| anyhow!("Invalid legacy address book peers"))?;
    let mut changed = false;
    for (id, recent) in &job.pending {
        if let Some(peer) = peers
            .iter_mut()
            .find(|peer| peer["id"].as_str() == Some(id))
        {
            for field in ["hash", "username", "hostname", "platform", "alias"] {
                if field == "alias" && !peer[field].as_str().unwrap_or_default().is_empty() {
                    continue;
                }
                if let Some(value) = recent.get(field).filter(|value| !value.is_empty()) {
                    if peer[field].as_str().unwrap_or_default() != value {
                        peer[field] = json!(value);
                        changed = true;
                    }
                }
            }
        } else if limit == 0 || peers.len() < limit {
            let mut peer = Map::new();
            for field in ["id", "hash", "username", "hostname", "platform", "alias"] {
                if let Some(value) = recent.get(field) {
                    peer.insert(field.to_owned(), json!(value));
                }
            }
            peer.insert("tags".to_owned(), json!([]));
            peers.push(Value::Object(peer));
            changed = true;
        }
    }
    if changed {
        request(
            account,
            job,
            Method::POST,
            "/api/ab",
            Some(json!({"data": data.to_string()})),
        )
        .await?;
    }
    complete(
        account,
        job,
        &job.pending.keys().cloned().collect::<Vec<_>>(),
        changed,
    )
}

pub(crate) fn start() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        tokio::spawn(run());
    });
}

async fn run() {
    let mut interval = tokio::time::interval(INTERVAL);
    loop {
        tokio::select! { _ = interval.tick() => {}, _ = HOST_CHANGED.notified() => {} }
        let jobs = (|| -> ResultType<Option<(Context, Vec<Stored>)>> {
            let mut host = HOST.lock().unwrap();
            load(&mut host)?;
            let Some(account) = context() else {
                return Ok(None);
            };
            let mut stored = host.stored.clone();
            stored.retain(|job| account.binding == job.binding);
            if !account.sync_enabled {
                for job in &mut stored {
                    job.book.enabled = false;
                    job.pending.clear();
                }
            }
            if stored != host.stored {
                save(&stored)?;
                host.stored = stored;
            }
            let jobs = host
                .stored
                .iter()
                .filter(|stored| stored.book.enabled && !stored.pending.is_empty())
                .cloned()
                .collect();
            Ok(Some((account, jobs)))
        })();
        match jobs {
            Ok(Some((account, jobs))) => {
                for job in jobs {
                    let result = if job.book.legacy {
                        synchronize_legacy(&account, &job).await
                    } else {
                        synchronize(&account, &job).await
                    };
                    if let Err(error) = result {
                        hbb_common::throttled_log!(
                            crate::common::API_LOG_INTERVAL,
                            warn,
                            "Background address book synchronization failed: {error}"
                        );
                    }
                }
            }
            Err(error) => hbb_common::throttled_log!(
                crate::common::API_LOG_INTERVAL,
                warn,
                "Background address book synchronization failed: {error}"
            ),
            _ => {}
        }
    }
}
