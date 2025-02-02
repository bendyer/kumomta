use crate::lifecycle::Activity;
use crate::logging::{log_disposition, LogDisposition, RecordType};
use crate::mod_kumo::{DefineSpoolParams, SpoolKind};
use crate::queue::QueueManager;
use anyhow::Context;
use chrono::Utc;
use message::Message;
use rfc5321::{EnhancedStatusCode, Response};
use spool::local_disk::LocalDiskSpool;
use spool::rocks::RocksSpool;
use spool::{Spool as SpoolTrait, SpoolEntry, SpoolId};
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::{Mutex, MutexGuard};
use tokio::task::JoinHandle;

lazy_static::lazy_static! {
    pub static ref MANAGER: Mutex<SpoolManager> = Mutex::new(SpoolManager::new());
}

#[derive(Clone)]
pub struct SpoolHandle(Arc<Spool>);

impl std::ops::Deref for SpoolHandle {
    type Target = dyn SpoolTrait + Send + Sync;
    fn deref(&self) -> &Self::Target {
        &*self.0.spool
    }
}

pub struct Spool {
    maintainer: StdMutex<Option<JoinHandle<()>>>,
    spool: Arc<dyn SpoolTrait + Send + Sync>,
}

impl std::ops::Deref for Spool {
    type Target = dyn SpoolTrait + Send + Sync;
    fn deref(&self) -> &Self::Target {
        &*self.spool
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        if let Some(handle) = self.maintainer.lock().unwrap().take() {
            handle.abort();
        }
    }
}

impl Spool {}

pub struct SpoolManager {
    named: HashMap<String, SpoolHandle>,
    spooled_in: bool,
}

impl SpoolManager {
    pub fn new() -> Self {
        Self {
            named: HashMap::new(),
            spooled_in: false,
        }
    }

    pub async fn get() -> MutexGuard<'static, Self> {
        MANAGER.lock().await
    }

    pub fn new_local_disk(&mut self, params: DefineSpoolParams) -> anyhow::Result<()> {
        tracing::debug!(
            "Defining local disk spool '{}' on {}",
            params.name,
            params.path.display()
        );
        self.named.insert(
            params.name.to_string(),
            SpoolHandle(Arc::new(Spool {
                maintainer: StdMutex::new(None),
                spool: match params.kind {
                    SpoolKind::LocalDisk => Arc::new(
                        LocalDiskSpool::new(&params.path, params.flush)
                            .with_context(|| format!("Opening spool {}", params.name))?,
                    ),
                    SpoolKind::RocksDB => Arc::new(
                        RocksSpool::new(&params.path, params.flush, params.rocks_params)
                            .with_context(|| format!("Opening spool {}", params.name))?,
                    ),
                },
            })),
        );
        Ok(())
    }

    #[allow(unused)]
    pub async fn get_named(name: &str) -> anyhow::Result<SpoolHandle> {
        Self::get().await.get_named_impl(name)
    }

    pub fn get_named_impl(&self, name: &str) -> anyhow::Result<SpoolHandle> {
        self.named
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no spool named '{name}' has been defined"))
    }

    pub fn spool_started(&self) -> bool {
        self.spooled_in
    }

    pub async fn remove_from_spool(id: SpoolId) -> anyhow::Result<()> {
        let (data_spool, meta_spool) = {
            let mgr = Self::get().await;
            (mgr.get_named_impl("data")?, mgr.get_named_impl("meta")?)
        };
        let res_data = data_spool.remove(id).await;
        let res_meta = meta_spool.remove(id).await;
        if let Err(err) = res_data {
            // We don't log at error level for these because that
            // is undesirable when deferred_spool is enabled.
            tracing::debug!("Error removing data for {id}: {err:#}");
        }
        if let Err(err) = res_meta {
            tracing::debug!("Error removing meta for {id}: {err:#}");
        }
        Ok(())
    }

    pub async fn remove_from_spool_impl(&mut self, id: SpoolId) -> anyhow::Result<()> {
        let data_spool = self.get_named_impl("data")?;
        let meta_spool = self.get_named_impl("meta")?;
        let res_data = data_spool.remove(id).await;
        let res_meta = meta_spool.remove(id).await;
        if let Err(err) = res_data {
            tracing::debug!("Error removing data for {id}: {err:#}");
        }
        if let Err(err) = res_meta {
            tracing::debug!("Error removing meta for {id}: {err:#}");
        }
        Ok(())
    }

    pub async fn start_spool(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.named.is_empty(), "No spools have been defined");

        let (tx, mut rx) = tokio::sync::mpsc::channel(32);

        for (name, spool) in self.named.iter_mut() {
            let is_meta = name == "meta";

            match name.as_str() {
                "meta" => spool::set_meta_spool(spool.0.spool.clone()),
                "data" => spool::set_data_spool(spool.0.spool.clone()),
                _ => {}
            }

            tracing::debug!("starting maintainer for spool {name} is_meta={is_meta}");

            let maintainer = crate::runtime::spawn(format!("maintain spool {name}"), {
                let name = name.clone();
                let spool = spool.clone();
                let tx = if is_meta { Some(tx.clone()) } else { None };
                {
                    async move {
                        // start enumeration
                        if let Some(tx) = tx {
                            tracing::info!("starting enumeration for {name}");
                            if let Err(err) = spool.enumerate(tx) {
                                tracing::error!(
                                    "error during spool enumeration for {name}: {err:#}"
                                );
                            }
                        }

                        // And maintain it every 10 minutes
                        loop {
                            tokio::time::sleep(Duration::from_secs(10 * 60)).await;
                            if let Err(err) = spool.cleanup().await {
                                tracing::error!("error doing spool cleanup for {name}: {err:#}");
                            }
                        }
                    }
                }
            })?;
            spool.0.maintainer.lock().unwrap().replace(maintainer);
        }

        // Ensure that there are no more senders outstanding,
        // otherwise we'll deadlock ourselves in the loop below
        drop(tx);

        let activity = Activity::get()?;
        let egress_source = None;
        let egress_pool = None;
        let mut spooled_in = 0;
        tracing::debug!("start_spool: waiting for enumeration");
        let mut config = config::load_config().await?;
        while let Some(entry) = rx.recv().await {
            if activity.is_shutting_down() {
                break;
            }
            let now = Utc::now();
            match entry {
                SpoolEntry::Item { id, data } => match Message::new_from_spool(id, data) {
                    Ok(msg) => {
                        spooled_in += 1;

                        config
                            .async_call_callback("spool_message_enumerated", msg.clone())
                            .await?;

                        match msg.get_queue_name() {
                            Ok(queue_name) => match QueueManager::resolve(&queue_name).await {
                                Err(err) => {
                                    tracing::error!(
                                        "failed to resolve queue {queue_name}: {err:#}"
                                    );
                                }
                                Ok(queue) => {
                                    let mut queue = queue.lock().await;

                                    let queue_config = queue.get_config();
                                    let max_age = queue_config.get_max_age();
                                    let age = msg.age(now);
                                    let num_attempts = queue_config.infer_num_attempts(age);
                                    msg.set_num_attempts(num_attempts);

                                    match queue_config.compute_delay_based_on_age(num_attempts, age)
                                    {
                                        None => {
                                            tracing::debug!("expiring {id} {age} > {max_age}");
                                            log_disposition(LogDisposition {
                                                kind: RecordType::Expiration,
                                                msg,
                                                site: "localhost",
                                                peer_address: None,
                                                response: Response {
                                                    code: 551,
                                                    enhanced_code: Some(EnhancedStatusCode {
                                                        class: 5,
                                                        subject: 4,
                                                        detail: 7,
                                                    }),
                                                    content: format!(
                                                        "Delivery time {age} > {max_age}"
                                                    ),
                                                    command: None,
                                                },
                                                egress_pool,
                                                egress_source,
                                                relay_disposition: None,
                                                delivery_protocol: None,
                                            })
                                            .await;
                                            self.remove_from_spool_impl(id).await?;
                                            continue;
                                        }
                                        Some(delay) => {
                                            msg.delay_by(delay).await?;
                                        }
                                    }

                                    if let Err(err) = queue.insert(msg).await {
                                        tracing::error!(
                                            "failed to insert Message {id} \
                                             to queue {queue_name}: {err:#}"
                                        );
                                        self.remove_from_spool_impl(id).await?;
                                    }
                                }
                            },
                            Err(err) => {
                                tracing::error!(
                                    "Message {id} failed to compute queue name!: {err:#}"
                                );
                                log_disposition(LogDisposition {
                                    kind: RecordType::Expiration,
                                    msg,
                                    site: "localhost",
                                    peer_address: None,
                                    response: Response {
                                        code: 551,
                                        enhanced_code: Some(EnhancedStatusCode {
                                            class: 5,
                                            subject: 1,
                                            detail: 3,
                                        }),
                                        content: format!("Failed to compute queue name: {err:#}"),
                                        command: None,
                                    },
                                    egress_pool,
                                    egress_source,
                                    relay_disposition: None,
                                    delivery_protocol: None,
                                })
                                .await;
                                self.remove_from_spool_impl(id).await?;
                            }
                        }
                    }
                    Err(err) => {
                        tracing::error!("Failed to parse metadata for {id}: {err:#}");
                        self.remove_from_spool_impl(id).await?;
                    }
                },
                SpoolEntry::Corrupt { id, error } => {
                    tracing::error!("Failed to load {id}: {error}");
                    // TODO: log this better
                    self.remove_from_spool_impl(id).await?;
                }
            }
        }
        self.spooled_in = true;
        tracing::debug!("start_spool: enumeration done, spooled in {spooled_in} msgs");
        Ok(())
    }
}
