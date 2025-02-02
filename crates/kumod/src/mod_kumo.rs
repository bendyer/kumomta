use crate::egress_path::EgressPathConfig;
use crate::egress_source::{EgressPool, EgressSource};
use crate::http_server::HttpListenerParams;
use crate::lifecycle::LifeCycle;
use crate::logging::{ClassifierParams, LogFileParams, LogHookParams};
use crate::queue::QueueConfig;
use crate::runtime::spawn;
use crate::smtp_server::{EsmtpDomain, EsmtpListenerParams, RejectError};
use anyhow::Context;
use config::{any_err, from_lua_value, get_or_create_module};
use mlua::{Function, Lua, LuaSerdeExt, Value};
use mod_redis::RedisConnKey;
use serde::Deserialize;
use spool::rocks::RocksSpoolParams;
use std::path::PathBuf;

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let kumo_mod = get_or_create_module(lua, "kumo")?;

    kumo_mod.set(
        "on",
        lua.create_function(move |lua, (name, func): (String, Function)| {
            let decorated_name = format!("kumomta-on-{}", name);

            let existing: Value = lua.named_registry_value(&decorated_name)?;
            match existing {
                Value::Nil => {}
                Value::Function(func) => {
                    let info = func.info();
                    let src = String::from_utf8_lossy(
                        info.source.as_ref().map(|v| v.as_slice()).unwrap_or(b"?"),
                    );
                    let line = info.line_defined;
                    return Err(mlua::Error::external(format!(
                        "{name} event already has a handler defined at {src}:{line}"
                    )));
                }
                _ => {
                    return Err(mlua::Error::external(format!(
                        "{name} event already has a handler"
                    )));
                }
            }

            lua.set_named_registry_value(&decorated_name, func)?;
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "set_diagnostic_log_filter",
        lua.create_function(move |_, filter: String| {
            crate::set_diagnostic_log_filter(&filter).map_err(any_err)
        })?,
    )?;

    kumo_mod.set(
        "configure_bounce_classifier",
        lua.create_function(move |lua, params: Value| {
            let params: ClassifierParams = from_lua_value(lua, params)?;
            params.register().map_err(any_err)
        })?,
    )?;

    kumo_mod.set(
        "configure_local_logs",
        lua.create_function(move |lua, params: Value| {
            let params: LogFileParams = from_lua_value(lua, params)?;
            crate::logging::Logger::init(params).map_err(any_err)
        })?,
    )?;

    kumo_mod.set(
        "configure_log_hook",
        lua.create_function(move |lua, params: Value| {
            let params: LogHookParams = from_lua_value(lua, params)?;
            crate::logging::Logger::init_hook(params).map_err(any_err)
        })?,
    )?;

    kumo_mod.set(
        "start_http_listener",
        lua.create_async_function(|lua, params: Value| async move {
            let params: HttpListenerParams = from_lua_value(lua, params)?;
            params.start().await.map_err(any_err)?;
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "start_esmtp_listener",
        lua.create_async_function(|lua, params: Value| async move {
            let params: EsmtpListenerParams = from_lua_value(lua, params)?;
            spawn("start_esmtp_listener", async move {
                if let Err(err) = params.run().await {
                    tracing::error!("Error in SmtpServer: {err:#}");
                }
            })
            .map_err(any_err)?;
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "define_spool",
        lua.create_async_function(|lua, params: Value| async move {
            let params = from_lua_value(lua, params)?;
            spawn("define_spool", async move {
                if let Err(err) = define_spool(params).await {
                    tracing::error!("Error in spool: {err:#}");
                    LifeCycle::request_shutdown().await;
                }
            })
            .map_err(any_err)?
            .await
            .map_err(any_err)
        })?,
    )?;

    kumo_mod.set(
        "configure_redis_throttles",
        lua.create_async_function(|lua, params: Value| async move {
            let key: RedisConnKey = from_lua_value(lua, params)?;
            let conn = key.open().await.map_err(any_err)?;
            throttle::use_redis(conn).map_err(any_err)
        })?,
    )?;

    kumo_mod.set(
        "reject",
        lua.create_function(move |_lua, (code, message): (u16, String)| {
            Err::<(), mlua::Error>(mlua::Error::external(RejectError { code, message }))
        })?,
    )?;

    kumo_mod.set(
        "make_listener_domain",
        lua.create_function(move |lua, params: Value| {
            let config: EsmtpDomain = from_lua_value(lua, params)?;
            Ok(config)
        })?,
    )?;

    kumo_mod.set(
        "make_egress_path",
        lua.create_function(move |lua, params: Value| {
            let config: EgressPathConfig = from_lua_value(lua, params)?;
            Ok(config)
        })?,
    )?;

    kumo_mod.set(
        "make_queue_config",
        lua.create_function(move |lua, params: Value| {
            let config: QueueConfig = from_lua_value(lua, params)?;
            Ok(config)
        })?,
    )?;

    kumo_mod.set(
        "make_egress_source",
        lua.create_function(move |lua, params: Value| {
            let source: EgressSource = from_lua_value(lua, params)?;
            Ok(source)
        })?,
    )?;

    kumo_mod.set(
        "make_egress_pool",
        lua.create_function(move |lua, params: Value| {
            let pool: EgressPool = from_lua_value(lua, params)?;
            // pool.register().map_err(any_err)
            Ok(pool)
        })?,
    )?;

    kumo_mod.set(
        "toml_load",
        lua.create_async_function(|lua, file_name: String| async move {
            let data = tokio::fs::read_to_string(&file_name)
                .await
                .with_context(|| format!("reading file {file_name}"))
                .map_err(any_err)?;

            let obj: toml::Value = toml::from_str(&data)
                .with_context(|| format!("parsing {file_name} as toml"))
                .map_err(any_err)?;
            Ok(lua.to_value(&obj))
        })?,
    )?;

    kumo_mod.set(
        "toml_parse",
        lua.create_function(move |lua, toml: String| {
            let obj: toml::Value = toml::from_str(&toml)
                .with_context(|| format!("parsing {toml} as toml"))
                .map_err(any_err)?;
            Ok(lua.to_value(&obj))
        })?,
    )?;

    kumo_mod.set(
        "toml_encode",
        lua.create_function(move |_lua, value: Value| toml::to_string(&value).map_err(any_err))?,
    )?;

    kumo_mod.set(
        "toml_encode_pretty",
        lua.create_function(move |_lua, value: Value| {
            toml::to_string_pretty(&value).map_err(any_err)
        })?,
    )?;

    kumo_mod.set(
        "json_load",
        lua.create_async_function(|lua, file_name: String| async move {
            let data = tokio::fs::read(&file_name)
                .await
                .with_context(|| format!("reading file {file_name}"))
                .map_err(any_err)?;

            let stripped = json_comments::StripComments::new(&*data);

            let obj: serde_json::Value = serde_json::from_reader(stripped)
                .with_context(|| format!("parsing {file_name} as json"))
                .map_err(any_err)?;
            Ok(lua.to_value(&obj))
        })?,
    )?;

    kumo_mod.set(
        "json_parse",
        lua.create_async_function(|lua, text: String| async move {
            let stripped = json_comments::StripComments::new(text.as_bytes());
            let obj: serde_json::Value = serde_json::from_reader(stripped)
                .with_context(|| format!("parsing {text} as json"))
                .map_err(any_err)?;
            Ok(lua.to_value(&obj))
        })?,
    )?;

    kumo_mod.set(
        "json_encode",
        lua.create_async_function(|_, value: Value| async move {
            serde_json::to_string(&value).map_err(any_err)
        })?,
    )?;
    kumo_mod.set(
        "json_encode_pretty",
        lua.create_async_function(|_, value: Value| async move {
            serde_json::to_string_pretty(&value).map_err(any_err)
        })?,
    )?;

    Ok(())
}

#[derive(Deserialize)]
pub enum SpoolKind {
    LocalDisk,
    RocksDB,
}
impl Default for SpoolKind {
    fn default() -> Self {
        Self::LocalDisk
    }
}

#[derive(Deserialize)]
pub struct DefineSpoolParams {
    pub name: String,
    pub path: PathBuf,
    #[serde(default)]
    pub kind: SpoolKind,
    #[serde(default)]
    pub flush: bool,
    #[serde(default)]
    pub rocks_params: Option<RocksSpoolParams>,
}

async fn define_spool(params: DefineSpoolParams) -> anyhow::Result<()> {
    crate::spool::SpoolManager::get()
        .await
        .new_local_disk(params)
}
