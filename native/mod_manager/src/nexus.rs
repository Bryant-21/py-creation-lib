use crate::{check_cancel, encoding, executables::text};
use md5::Md5;
use pyo3::prelude::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use unicode_casefold::UnicodeCaseFold;

pyo3::create_exception!(
    mod_manager_native,
    NexusFailure,
    pyo3::exceptions::PyValueError
);
pub fn failure(message: impl Into<String>) -> PyErr {
    NexusFailure::new_err((message.into(), 3600, Option::<u16>::None))
}
pub const GAMES: &[&str] = &[
    "fallout4",
    "fallout76",
    "skyrimspecialedition",
    "skyrim",
    "fallout3",
    "newvegas",
    "starfield",
];
fn domain(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn identifier(value: &str) -> Option<u64> {
    if value.is_empty()
        || value.len() > 12
        || value.starts_with('0')
        || !value.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    value.parse().ok()
}

#[pyfunction]
pub fn nexus_game_domain(name: &str) -> String {
    let compact: String = name
        .case_fold()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect();
    match compact.as_str() {
        "fo4" => "fallout4".into(),
        "fo76" => "fallout76".into(),
        "skyrimse" => "skyrimspecialedition".into(),
        "fo3" => "fallout3".into(),
        "fnv" | "falloutnewvegas" => "newvegas".into(),
        value if GAMES.contains(&value) => compact,
        _ => String::new(),
    }
}

#[pyfunction]
fn nexus_parse_nxm(raw: &str) -> PyResult<String> {
    if raw.len() > 4096
        || raw
            .chars()
            .any(|c| c.is_whitespace() || c < ' ' || c == '\u{7f}' || "\"<>".contains(c))
    {
        return Err(failure("Invalid Nexus browser link."));
    }
    let invalid = || {
        failure(
            "Invalid Nexus mod/file link. Collection links and other URL forms are not supported.",
        )
    };
    let (scheme, rest) = raw.split_once("://").ok_or_else(invalid)?;
    let (game, rest) = rest.split_once('/').ok_or_else(invalid)?;
    if !scheme.eq_ignore_ascii_case("nxm") || !domain(game) || rest.contains('#') {
        return Err(invalid());
    }
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() != 4 || parts[0] != "mods" || parts[2] != "files" {
        return Err(invalid());
    }
    let mod_id = identifier(parts[1]).ok_or_else(invalid)?;
    let file_id = identifier(parts[3]).ok_or_else(invalid)?;
    let mut parameters = BTreeMap::new();
    if !query.is_empty() {
        let fields: Vec<_> = query.split('&').collect();
        if fields.len() > 3 || fields.iter().any(|v| !v.contains('=')) {
            return Err(invalid());
        }
        for (name, value) in url::form_urlencoded::parse(query.as_bytes()) {
            if !matches!(name.as_ref(), "key" | "expires" | "user_id")
                || value.is_empty()
                || parameters
                    .insert(name.into_owned(), value.into_owned())
                    .is_some()
            {
                return Err(invalid());
            }
        }
    }
    if parameters.contains_key("key") != parameters.contains_key("expires") {
        return Err(invalid());
    }
    let key = parameters.get("key").map(String::as_str).unwrap_or("");
    if key.len() > 1024
        || key
            .bytes()
            .any(|b| !b.is_ascii_alphanumeric() && !b"_+-=".contains(&b))
    {
        return Err(invalid());
    }
    let expires = parameters
        .get("expires")
        .map(|v| identifier(v).ok_or_else(invalid))
        .transpose()?
        .unwrap_or(0);
    let user_id = parameters
        .get("user_id")
        .map(|v| identifier(v).ok_or_else(invalid))
        .transpose()?
        .unwrap_or(0);
    Ok(json!({"game":game,"mod_id":mod_id,"file_id":file_id,"key":key,"expires":expires,"user_id":user_id}).to_string())
}

#[pyfunction]
pub fn nexus_cdn_url(raw: &str) -> PyResult<String> {
    let invalid =
        || failure("Nexus returned a download destination outside the approved HTTPS CDN hosts.");
    if raw.chars().any(|c| c.is_whitespace() || c < ' ') {
        return Err(invalid());
    }
    let url = url::Url::parse(raw).map_err(|_| invalid())?;
    let host = url.host_str().unwrap_or("");
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|v| v != 443)
        || !["nexusmods.com", "nexus-cdn.com"]
            .iter()
            .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
    {
        return Err(invalid());
    }
    Ok(raw.into())
}

#[pyfunction]
pub(crate) fn nexus_key(api_key: &str) -> PyResult<()> {
    if api_key.is_empty()
        || api_key.len() > 4096
        || !api_key.bytes().all(|b| (0x21..=0x7e).contains(&b))
    {
        return Err(failure("Enter your Nexus API key for this session."));
    }
    Ok(())
}

#[pyfunction]
fn nexus_expiry(expires: f64, now: f64) -> PyResult<()> {
    if expires != 0.0 && expires <= now {
        return Err(failure(
            "This browser download link expired. Request a fresh Mod Manager Download link from Nexus.",
        ));
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn nexus_http_error(status: u16, retry: &str) -> PyResult<()> {
    let retry = retry
        .parse::<u64>()
        .map(|v| v.clamp(60, 86400))
        .unwrap_or(3600);
    let message = match status {
        401 => "Nexus authentication failed. Reconnect with a valid session API key.".into(),
        403 => {
            "Nexus denied this request. Check the link's account, expiry, and download entitlement."
                .into()
        }
        404 => "This Nexus mod or file is unavailable or was removed.".into(),
        429 => "Nexus rate limit reached. Automatic checks will back off.".into(),
        _ => format!("Nexus request failed (HTTP {status})."),
    };
    Err(NexusFailure::new_err((message, retry, Some(status))))
}

pub fn mod_identity(game: &str, id: &Value) -> PyResult<()> {
    if !GAMES.contains(&game)
        || id
            .as_u64()
            .is_none_or(|id| id == 0 || id >= 1_000_000_000_000)
    {
        return Err(failure("This mod has no supported Nexus identity."));
    }
    Ok(())
}

#[pyfunction]
fn nexus_mod_identity(game: &str, id: &str) -> PyResult<()> {
    mod_identity(game, &encoding::parse(id)?)
}

#[pyfunction]
pub(crate) fn nexus_json_request(
    key: &str,
    version: &str,
    endpoint: &str,
    parameters: &str,
    method: &str,
    body: &str,
) -> PyResult<String> {
    if key.is_empty() {
        return Err(failure("Connect a Nexus account for this session first."));
    }
    if !matches!(method, "GET" | "POST" | "DELETE") {
        return Err(failure("Unsupported Nexus API operation."));
    }
    let parameters = encoding::parse(parameters)?;
    let body = encoding::parse(body)?;
    let mut url = format!("https://api.nexusmods.com/v1/{endpoint}");
    if let Some(parameters) = parameters.as_object().filter(|p| !p.is_empty()) {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        for (name, value) in parameters {
            query.append_pair(
                name,
                &value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string()),
            );
        }
        url.push('?');
        url.push_str(&query.finish());
    }
    let mut headers = json!({"apikey":key,"Application-Name":"ModBox21","Application-Version":version,"User-Agent":format!("ModBox21/{version}"),"Protocol-Version":"1.0.0","Accept":"application/json"});
    if !body.is_null() {
        headers["Content-Type"] = json!("application/json");
    }
    Ok(json!({"url":url,"headers":headers,"method":method,"data":if body.is_null() { Value::Null } else {json!(encoding::encode(&body,true,true))}}).to_string())
}

#[pyfunction]
pub(crate) fn nexus_json_response(data: &[u8], status: u16, allow_empty: bool) -> PyResult<String> {
    if status == 202 {
        return Err(failure(
            "Nexus accepted the request but its outcome is unconfirmed. Refresh account state before retrying.",
        ));
    }
    if data.len() > 2 * 1024 * 1024 {
        return Err(failure("Nexus API response exceeds the safety limit."));
    }
    if allow_empty && data.iter().all(u8::is_ascii_whitespace) {
        return Ok("null".into());
    }
    serde_json::from_slice::<Value>(data)
        .map(|v| v.to_string())
        .map_err(|_| failure("Nexus returned an invalid API response."))
}

fn request(
    callback: &Bound<'_, PyAny>,
    endpoint: &str,
    parameters: Value,
    method: &str,
    body: Value,
    allow_empty: bool,
) -> PyResult<Value> {
    let kwargs = pyo3::types::PyDict::new(callback.py());
    kwargs.set_item("method", method)?;
    kwargs.set_item("allow_empty", allow_empty)?;
    let json = callback.py().import("json")?;
    kwargs.set_item("body", json.call_method1("loads", (body.to_string(),))?)?;
    let value = callback.call(
        (
            endpoint,
            json.call_method1("loads", (parameters.to_string(),))?,
        ),
        Some(&kwargs),
    )?;
    encoding::parse(&json.call_method1("dumps", (value,))?.extract::<String>()?)
}

#[pyfunction]
fn nexus_operation(action: &str, arguments: &str, callback: &Bound<'_, PyAny>) -> PyResult<String> {
    let args = encoding::parse(arguments)?;
    let game = text(&args, "game");
    let id = &args["mod_id"];
    if matches!(action, "files" | "track" | "endorse" | "abstain") {
        mod_identity(game, id)?;
    }
    let result = match action {
        "categories" => {
            if !domain(game) {
                return Err(failure("Choose a valid Nexus game domain."));
            }
            let game = request(
                callback,
                &format!("games/{game}.json"),
                Value::Null,
                "GET",
                Value::Null,
                false,
            )?;
            game.get("categories")
                .cloned()
                .ok_or_else(|| failure("Nexus returned no game category catalog."))?
        }
        "connect" => {
            let user = request(
                callback,
                "users/validate.json",
                Value::Null,
                "GET",
                Value::Null,
                false,
            )?;
            if !user.is_object() || user["user_id"].as_i64().is_none() {
                return Err(failure("Nexus did not return a valid account identity."));
            }
            let name = user
                .get("name")
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| v.to_string())
                })
                .unwrap_or_else(|| "Nexus user".into())
                .chars()
                .take(100)
                .collect::<String>();
            json!({"user_id":user["user_id"],"name":name,"premium":user["is_premium"].as_bool().unwrap_or(false)})
        }
        "mod_info" => {
            if !domain(game) || id.as_i64().is_none_or(|id| id <= 0) {
                return Err(failure("This mod has no supported Nexus identity."));
            }
            let value = request(
                callback,
                &format!("games/{game}/mods/{id}.json"),
                Value::Null,
                "GET",
                Value::Null,
                false,
            )?;
            if !value.is_object() {
                return Err(failure("Nexus mod metadata is invalid."));
            }
            value
        }
        "files" => {
            let value = request(
                callback,
                &format!("games/{game}/mods/{id}/files.json"),
                Value::Null,
                "GET",
                Value::Null,
                false,
            )?;
            if !value.is_object() {
                return Err(failure("Nexus file inventory is invalid."));
            }
            value
        }
        "tracked_mods" | "endorsements" => {
            let value = request(
                callback,
                &format!("user/{action}.json"),
                Value::Null,
                "GET",
                Value::Null,
                false,
            )?;
            if value.as_array().is_none_or(|v| v.len() > 20_000) {
                return Err(failure(if action == "tracked_mods" {
                    "Nexus tracked-mod inventory is invalid or exceeds its safety limit."
                } else {
                    "Nexus endorsement inventory is invalid or exceeds its safety limit."
                }));
            }
            value
        }
        "track" => request(
            callback,
            "user/tracked_mods.json",
            Value::Null,
            if args["enabled"] == false {
                "DELETE"
            } else {
                "POST"
            },
            json!({"domain_name":game,"mod_id":id}),
            true,
        )?,
        "endorse" | "abstain" => {
            let version = args["version"]
                .as_str()
                .ok_or_else(|| failure("The installed Nexus version metadata is invalid."))?;
            if version.chars().count() > 256 || version.chars().any(|c| c < ' ') {
                return Err(failure("The installed Nexus version metadata is invalid."));
            }
            request(
                callback,
                &format!("games/{game}/mods/{id}/{action}.json"),
                Value::Null,
                "POST",
                json!({"Version":version}),
                true,
            )?
        }
        "resolve" => {
            nexus_expiry(
                args["expires"].as_f64().unwrap_or(0.0),
                args["now"].as_f64().unwrap_or(0.0),
            )?;
            let user = &args["user"];
            if !user.is_object() || user.as_object().unwrap().is_empty() {
                return Err(failure("Connect a Nexus account for this session first."));
            }
            let user_id = args["user_id"].as_u64().unwrap_or(0);
            if user_id != 0 && args["user_id"] != user["user_id"] {
                return Err(failure(
                    "This browser link belongs to another Nexus account. Generate a new link while signed into the connected account.",
                ));
            }
            if user["premium"] != true
                && (text(&args, "key").is_empty()
                    || args["expires"].as_u64().unwrap_or(0) == 0
                    || user_id == 0)
            {
                return Err(failure(
                    "Free accounts need a fresh Mod Manager Download link including its account and expiry parameters.",
                ));
            }
            let base = format!("games/{game}/mods/{id}/files/{}", args["file_id"]);
            let metadata = request(
                callback,
                &format!("{base}.json"),
                Value::Null,
                "GET",
                Value::Null,
                false,
            )?;
            let filename = metadata["file_name"]
                .as_str()
                .ok_or_else(|| failure("Nexus file metadata is incomplete."))?;
            let filename = crate::relative_path(filename, false)?;
            if filename.contains('/')
                || !["zip", "7z", "rar", "fomod"].contains(
                    &filename
                        .rsplit('.')
                        .next()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .as_str(),
                )
            {
                return Err(failure(
                    "Nexus file must have a safe mod-archive filename (ZIP, 7z, RAR, or FOMOD).",
                ));
            }
            let parameters = if text(&args, "key").is_empty() {
                Value::Null
            } else {
                json!({"key":args["key"],"expires":args["expires"]})
            };
            let mirrors = request(
                callback,
                &format!("{base}/download_link.json"),
                parameters,
                "GET",
                Value::Null,
                false,
            )?;
            let uri = mirrors
                .as_array()
                .and_then(|v| v.first())
                .and_then(|v| v["URI"].as_str())
                .ok_or_else(|| failure("Nexus did not provide an available download mirror."))?;
            json!([metadata, nexus_cdn_url(uri)?])
        }
        _ => return Err(failure("Unsupported Nexus API operation.")),
    };
    Ok(result.to_string())
}

#[pyfunction]
#[pyo3(signature = (response, destination, md5, cancel=None, progress=None))]
pub(crate) fn nexus_download(
    response: &Bound<'_, PyAny>,
    destination: PathBuf,
    md5: &str,
    cancel: Option<&Bound<'_, PyAny>>,
    progress: Option<&Bound<'_, PyAny>>,
) -> PyResult<(u64, String)> {
    let total: String = response
        .getattr("headers")?
        .call_method1("get", ("Content-Length", ""))?
        .extract()?;
    let total = total.parse::<u64>().unwrap_or(0);
    let mut writer = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    if total > 8 * 1024u64.pow(3) {
        return Err(failure("Nexus archive exceeds the 8-GiB download limit."));
    }
    let mut copied = 0u64;
    let mut digest = Sha256::new();
    let mut checksum = Md5::new();
    loop {
        check_cancel(cancel)?;
        let chunk = response.call_method1("read", (1024 * 1024,)).map_err(|_| {
            failure("Nexus response was interrupted. Check the connection and retry.")
        })?;
        let chunk = chunk.cast::<pyo3::types::PyBytes>()?.as_bytes();
        if chunk.is_empty() {
            break;
        }
        copied += chunk.len() as u64;
        if copied > 8 * 1024u64.pow(3) || (total != 0 && copied > total) {
            return Err(failure(
                "Download size exceeded its advertised or configured limit.",
            ));
        }
        writer.write_all(chunk)?;
        digest.update(chunk);
        checksum.update(chunk);
        if let Some(progress) = progress {
            let digits = copied.to_string();
            let mut formatted = String::new();
            for (i, c) in digits.chars().enumerate() {
                if i > 0 && (digits.len() - i) % 3 == 0 {
                    formatted.push(',');
                }
                formatted.push(c);
            }
            progress.call1((
                if total == 0 {
                    None
                } else {
                    Some(copied as f64 / total as f64)
                },
                format!("{formatted} bytes downloaded"),
            ))?;
        }
    }
    if copied == 0 || (total != 0 && copied != total) {
        return Err(failure(
            "Download ended before the complete archive was received.",
        ));
    }
    if !md5.is_empty()
        && (md5.len() != 32
            || !md5.bytes().all(|b| b.is_ascii_hexdigit())
            || format!("{:x}", checksum.finalize()) != md5.to_ascii_lowercase())
    {
        return Err(failure(
            "Downloaded archive failed the Nexus file checksum.",
        ));
    }
    check_cancel(cancel)?;
    Ok((copied, format!("{:x}", digest.finalize())))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("NexusFailure", m.py().get_type::<NexusFailure>())?;
    m.add_function(wrap_pyfunction!(nexus_game_domain, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_parse_nxm, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_cdn_url, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_key, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_expiry, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_http_error, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_mod_identity, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_json_request, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_json_response, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_operation, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_download, m)?)?;
    Ok(())
}
