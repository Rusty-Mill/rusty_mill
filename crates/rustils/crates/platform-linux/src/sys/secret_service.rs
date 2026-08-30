//! The Secret Service API (`org.freedesktop.secrets`, RFC v2 R5+, D15,
//! Phase 6 item 2, rustils#78) over `sys::dbus`'s transport — the real
//! Linux `CredentialStore` implementation.
//!
//! A fresh [`Connection`] (and D-Bus session, and Secret Service
//! session) is opened for every call — no persistent connection state,
//! matching how the Windows backend also makes a fresh `CredWriteW`/
//! `CredReadW` call each time rather than holding a handle open. This
//! trades a little overhead for not having to manage reconnection,
//! thread-safety of a shared long-lived socket, or a background
//! session-keepalive — none of which any named case for this slice
//! needs.
//!
//! Reachability failures (no D-Bus session bus, no Secret Service
//! provider registered, no default collection, or a collection that's
//! locked and can't be unlocked non-interactively — this is a
//! non-interactive backend with no window handle to complete a `Prompt`
//! with) are distinguished from genuine protocol errors: [`available`]
//! reports [`CredentialStoreStatus::Unavailable`] for them, and
//! [`get`]/[`set`] surface them as a real `Err` (not a silent
//! `Ok(None)`/`Ok(())`) — per the portable trait's own contract
//! (`platform::security::CredentialStore`'s doc comment): a clean miss
//! is "nothing stored under this name," which is a different, weaker
//! claim than "the whole store isn't reachable right now." A caller
//! that wants the softer behavior checks `available()` first, exactly
//! as that method exists to let it.
//!
//! `available_at`/`get_at`/`set_at` connect to one explicit D-Bus
//! address rather than discovering the session bus, mirroring
//! `sys::dbus::Connection::connect_to` — this crate's own integration
//! tests use them so a test-spawned `dbus-daemon`/`gnome-keyring-daemon`
//! pair never has to mutate the process-wide `DBUS_SESSION_BUS_ADDRESS`
//! environment variable (unsound under parallel test threads).

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::security::CredentialStoreStatus;

use super::dbus::{Connection, Value};

const SERVICE_DEST: &str = "org.freedesktop.secrets";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SERVICE_IFACE: &str = "org.freedesktop.Secret.Service";
const COLLECTION_IFACE: &str = "org.freedesktop.Secret.Collection";
const ITEM_IFACE: &str = "org.freedesktop.Secret.Item";
const PROPERTIES_IFACE: &str = "org.freedesktop.DBus.Properties";
/// D-Bus's null-object-path sentinel — `Unlock`/`CreateItem` use it to
/// mean "no prompt needed", `ReadAlias` uses it to mean "no such alias".
const NULL_PATH: &str = "/";

fn proto_err(detail: &'static str) -> PlatformError {
    PlatformError::new(ErrorKind::InvalidInput, OsCode::None, detail)
}

fn unreachable_err() -> PlatformError {
    PlatformError::new(
        ErrorKind::Other,
        OsCode::None,
        "the Secret Service backing store is not currently reachable \
         (check CredentialStore::available() first)",
    )
}

/// Whether `e` represents "the service isn't there right now" (no bus,
/// nothing listening, or a D-Bus-level `ServiceUnknown`) rather than a
/// genuine protocol/logic error.
fn is_benign_unreachable(e: &PlatformError) -> bool {
    if matches!(e.kind, ErrorKind::ConnectionRefused | ErrorKind::NotFound) {
        return true;
    }
    e.kind == ErrorKind::Other
        && e.path.as_deref().and_then(|p| p.to_str())
            == Some("org.freedesktop.DBus.Error.ServiceUnknown")
}

fn attributes_value(service: &str, account: &str) -> Value {
    Value::Array(
        "{ss}".into(),
        vec![
            Value::DictEntry(
                Box::new(Value::String("service".into())),
                Box::new(Value::String(service.into())),
            ),
            Value::DictEntry(
                Box::new(Value::String("account".into())),
                Box::new(Value::String(account.into())),
            ),
        ],
    )
}

fn get_property(conn: &mut Connection, path: &str, iface: &str, prop: &str) -> Result<Value> {
    let reply = conn.call(
        SERVICE_DEST,
        path,
        PROPERTIES_IFACE,
        "Get",
        "ss",
        vec![Value::String(iface.into()), Value::String(prop.into())],
    )?;
    match reply.body.into_iter().next() {
        Some(Value::Variant(v)) => Ok(*v),
        _ => Err(proto_err(
            "Properties.Get did not return the expected variant",
        )),
    }
}

/// Open a Secret Service session on an already-connected `conn`,
/// resolve the default collection, and unlock it if needed. `Ok(None)`
/// for any benign reachability failure (see this module's own doc
/// comment); `Err` only for a genuine protocol-shape surprise.
fn prepare_session_over(mut conn: Connection) -> Result<Option<(Connection, String, String)>> {
    let open = match conn.call(
        SERVICE_DEST,
        SERVICE_PATH,
        SERVICE_IFACE,
        "OpenSession",
        "sv",
        vec![
            Value::String("plain".into()),
            Value::Variant(Box::new(Value::String(String::new()))),
        ],
    ) {
        Ok(m) => m,
        Err(e) if is_benign_unreachable(&e) => return Ok(None),
        Err(e) => return Err(e),
    };
    let Some(Value::ObjectPath(session_path)) = open.body.get(1) else {
        return Err(proto_err(
            "OpenSession did not return the expected session path",
        ));
    };
    let session_path = session_path.clone();

    let read_alias = conn.call(
        SERVICE_DEST,
        SERVICE_PATH,
        SERVICE_IFACE,
        "ReadAlias",
        "s",
        vec![Value::String("default".into())],
    )?;
    let Some(Value::ObjectPath(collection_path)) = read_alias.body.first() else {
        return Err(proto_err(
            "ReadAlias did not return the expected object path",
        ));
    };
    if collection_path == NULL_PATH {
        return Ok(None); // No default collection configured on this system.
    }
    let collection_path = collection_path.clone();

    let locked = get_property(&mut conn, &collection_path, COLLECTION_IFACE, "Locked")?;
    if matches!(locked, Value::Boolean(true)) {
        let unlock = conn.call(
            SERVICE_DEST,
            SERVICE_PATH,
            SERVICE_IFACE,
            "Unlock",
            "ao",
            vec![Value::Array(
                "o".into(),
                vec![Value::ObjectPath(collection_path.clone())],
            )],
        )?;
        let (Some(Value::Array(_, unlocked)), Some(Value::ObjectPath(prompt))) =
            (unlock.body.first(), unlock.body.get(1))
        else {
            return Err(proto_err("Unlock did not return the expected reply shape"));
        };
        let actually_unlocked = unlocked
            .iter()
            .any(|v| matches!(v, Value::ObjectPath(p) if p == &collection_path));
        if !actually_unlocked || prompt != NULL_PATH {
            // Needs an interactive prompt — this is a non-interactive
            // backend with no window handle to complete one.
            return Ok(None);
        }
    }

    Ok(Some((conn, session_path, collection_path)))
}

fn prepare_session() -> Result<Option<(Connection, String, String)>> {
    match Connection::session() {
        Ok(conn) => prepare_session_over(conn),
        Err(e) if is_benign_unreachable(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

fn prepare_session_at(address: &str) -> Result<Option<(Connection, String, String)>> {
    match Connection::connect_to(address) {
        Ok(conn) => prepare_session_over(conn),
        Err(e) if is_benign_unreachable(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

fn get_with(
    mut conn: Connection,
    session_path: String,
    collection_path: String,
    service: &str,
    account: &str,
) -> Result<Option<Vec<u8>>> {
    let search = conn.call(
        SERVICE_DEST,
        &collection_path,
        COLLECTION_IFACE,
        "SearchItems",
        "a{ss}",
        vec![attributes_value(service, account)],
    )?;
    let Some(Value::Array(_, items)) = search.body.first() else {
        return Err(proto_err("SearchItems did not return the expected array"));
    };
    let Some(Value::ObjectPath(item_path)) = items.first() else {
        return Ok(None);
    };
    let item_path = item_path.clone();

    let secret_reply = conn.call(
        SERVICE_DEST,
        &item_path,
        ITEM_IFACE,
        "GetSecret",
        "o",
        vec![Value::ObjectPath(session_path)],
    )?;
    let Some(Value::Struct(fields)) = secret_reply.body.first() else {
        return Err(proto_err(
            "GetSecret did not return the expected Secret struct",
        ));
    };
    // Secret = (session, parameters: ay, value: ay, content_type: s) —
    // `value` is field index 2.
    let Some(Value::Array(_, byte_values)) = fields.get(2) else {
        return Err(proto_err(
            "Secret struct did not contain the expected value byte array",
        ));
    };
    let bytes = byte_values
        .iter()
        .map(|v| match v {
            Value::Byte(b) => Ok(*b),
            _ => Err(proto_err("Secret value array contained a non-byte element")),
        })
        .collect::<Result<Vec<u8>>>()?;
    Ok(Some(bytes))
}

fn set_with(
    mut conn: Connection,
    session_path: String,
    collection_path: String,
    service: &str,
    account: &str,
    secret: &[u8],
) -> Result<()> {
    let properties = Value::Array(
        "{sv}".into(),
        vec![
            Value::DictEntry(
                Box::new(Value::String("org.freedesktop.Secret.Item.Label".into())),
                Box::new(Value::Variant(Box::new(Value::String(format!(
                    "rustils: {service} ({account})"
                ))))),
            ),
            Value::DictEntry(
                Box::new(Value::String(
                    "org.freedesktop.Secret.Item.Attributes".into(),
                )),
                Box::new(Value::Variant(Box::new(attributes_value(service, account)))),
            ),
        ],
    );
    let secret_struct = Value::Struct(vec![
        Value::ObjectPath(session_path),
        Value::Array("y".into(), vec![]),
        Value::Array("y".into(), secret.iter().map(|b| Value::Byte(*b)).collect()),
        Value::String("text/plain".into()),
    ]);

    let reply = conn.call(
        SERVICE_DEST,
        &collection_path,
        COLLECTION_IFACE,
        "CreateItem",
        "a{sv}(oayays)b",
        vec![properties, secret_struct, Value::Boolean(true)],
    )?;
    let (Some(Value::ObjectPath(_)), Some(Value::ObjectPath(prompt))) =
        (reply.body.first(), reply.body.get(1))
    else {
        return Err(proto_err(
            "CreateItem did not return the expected reply shape",
        ));
    };
    if prompt != NULL_PATH {
        return Err(proto_err(
            "CreateItem unexpectedly required an interactive prompt",
        ));
    }
    Ok(())
}

/// Check reachability without touching any item data.
pub fn available() -> CredentialStoreStatus {
    status_of(prepare_session())
}

fn status_of(r: Result<Option<(Connection, String, String)>>) -> CredentialStoreStatus {
    match r {
        Ok(Some(_)) => CredentialStoreStatus::Available,
        Ok(None) | Err(_) => CredentialStoreStatus::Unavailable,
    }
}

/// The stored secret for `(service, account)`, or `Ok(None)` if nothing
/// is stored under that exact attribute pair in the default collection.
pub fn get(service: &str, account: &str) -> Result<Option<Vec<u8>>> {
    let (conn, sp, cp) = prepare_session()?.ok_or_else(unreachable_err)?;
    get_with(conn, sp, cp, service, account)
}

/// Store `secret` under `(service, account)` in the default collection,
/// replacing any existing item with the same attribute pair
/// (`CreateItem`'s own `replace` parameter — no separate search-then-
/// delete needed).
pub fn set(service: &str, account: &str, secret: &[u8]) -> Result<()> {
    let (conn, sp, cp) = prepare_session()?.ok_or_else(unreachable_err)?;
    set_with(conn, sp, cp, service, account, secret)
}

/// [`available`], connecting to `address` instead of discovering the
/// session bus — see this module's own doc comment.
pub fn available_at(address: &str) -> CredentialStoreStatus {
    status_of(prepare_session_at(address))
}

/// [`get`], connecting to `address` instead of discovering the session
/// bus.
pub fn get_at(address: &str, service: &str, account: &str) -> Result<Option<Vec<u8>>> {
    let (conn, sp, cp) = prepare_session_at(address)?.ok_or_else(unreachable_err)?;
    get_with(conn, sp, cp, service, account)
}

/// [`set`], connecting to `address` instead of discovering the session
/// bus.
pub fn set_at(address: &str, service: &str, account: &str, secret: &[u8]) -> Result<()> {
    let (conn, sp, cp) = prepare_session_at(address)?.ok_or_else(unreachable_err)?;
    set_with(conn, sp, cp, service, account, secret)
}
