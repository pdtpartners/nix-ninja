//! Sync client for the Nix daemon's worker protocol, backed by
//! `harmonia_store_remote::pool::ConnectionPool` for concurrent acquisition
//! across worker threads. Inside a `builder-rpc-v0` sandbox
//! (NixOS/nix#15793), the daemon socket is exposed via `$NIX_REMOTE` and
//! only a small allowlist is permitted: `Add{ToStore,ToStoreNar,TextToStore}`
//! plus `SubmitOutput`. Outside the sandbox the same client talks to the
//! standard daemon socket.

pub mod aterm;

use std::io::Cursor;
use std::path::{Path, PathBuf};

use futures::StreamExt;
use harmonia_protocol::store_path::StoreDir;
use harmonia_protocol::types::{DaemonError, DaemonStore};
use harmonia_store_core::derivation::Derivation;
use harmonia_store_core::derived_path::{OutputName, SingleDerivedPath};
use harmonia_store_core::store_path::{ContentAddressMethodAlgorithm, StorePath, StorePathSet};
use harmonia_store_remote::{ConnectionPool, PoolConfig};
use tokio::io::BufReader;
use tokio::runtime::{Builder as RuntimeBuilder, Runtime};

/// Env var the daemon sets inside a `builder-rpc-v0` sandbox.
pub const SOCKET_ENV: &str = "NIX_REMOTE";

/// Fallback for when `$NIX_REMOTE` is unset — Nix's standard daemon socket path.
const DEFAULT_DAEMON_SOCKET: &str = "/nix/var/nix/daemon-socket/socket";

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("daemon error: {0}")]
    Daemon(#[from] DaemonError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("$NIX_REMOTE has unsupported scheme: {0}")]
    UnsupportedRemote(String),
    #[error("nar encode: {0}")]
    Nar(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct BuilderRpcClient {
    /// Multi-thread so the pool's RAII drop tasks can complete asynchronously.
    runtime: Runtime,
    pool: ConnectionPool,
    /// Whether or not nix-ninja is running within a derivation
    /// Needed since scanning is not available outside derivations
    in_drv: bool,
}

impl BuilderRpcClient {
    /// Connect to `$NIX_REMOTE` if set, otherwise the standard daemon
    /// socket. Returns `Ok(None)` when no socket exists.
    pub fn connect_from_env() -> Result<Option<Self>> {
        let path = match std::env::var(SOCKET_ENV) {
            Ok(remote) => parse_unix_remote(&remote)?,
            Err(_) => PathBuf::from(DEFAULT_DAEMON_SOCKET),
        };
        if !path.exists() {
            return Ok(None);
        }

        // There is no simple OS flag to tell if we are running in a derivation,
        // but NIX_BUILD_TOP is always set by the nix daemon before building
        // and has no purpose outside a nix derivation.
        let in_drv = std::env::var_os("NIX_BUILD_TOP").is_some();

        Some(Self::connect_unix(&path, in_drv)).transpose()
    }

    pub fn connect_unix(path: &Path, in_drv: bool) -> Result<Self> {
        let runtime = RuntimeBuilder::new_multi_thread().enable_all().build()?;
        let pool = ConnectionPool::new(path, PoolConfig::default());
        Ok(Self {
            runtime,
            pool,
            in_drv,
        })
    }

    // Serialise a derivation and add it to the store.
    // Should be preferred to `add_to_store_text` for derivations,
    pub fn add_drv_to_store(
        &self,
        store_dir: &StoreDir,
        drv: &Derivation,
    ) -> Result<(StorePath, Vec<u8>)> {
        let bytes = aterm::print_derivation_aterm(store_dir, drv).into_bytes();
        let refs: StorePathSet = drv.inputs.iter().map(|p| p.root_path().clone()).collect();
        let name = format!("{}.drv", drv.name);
        let info = self.runtime.block_on(async {
            let mut guard = self.pool.acquire().await?;
            let source = BufReader::new(Cursor::new(bytes.clone()));
            await_result(guard.client().add_ca_to_store(
                &name,
                ContentAddressMethodAlgorithm::Text,
                &refs,
                false,
                source,
            ))
            .await
        })?;
        Ok((info.path, bytes))
    }

    /// Add bytes as a text-CA store object.
    /// Use for small files, but never for derivations.
    /// Derivations have different reference scanning logic, implemented in the
    /// `add_drv_to_store` function
    pub fn add_to_store_text(&self, name: &str, bytes: &[u8]) -> Result<StorePath> {
        let info = self.runtime.block_on(async {
            let mut guard = self.pool.acquire().await?;
            let source = BufReader::new(Cursor::new(bytes));
            if self.in_drv {
                await_result(guard.client().add_to_store_scanning(
                    name,
                    ContentAddressMethodAlgorithm::Text,
                    source,
                ))
                .await
            } else {
                // Unfortunately outside of a derivation we do not have a good
                // idea of possible paths for which we can scan.
                // A trivial "scan for all paths" implementation would include nonexistent paths
                // from documentation and fail on important projects, e.g. nix.
                // An empty reference set is identical to the fallback `nix store add` case
                // and preferable to no running outside a derivation at all.
                await_result(guard.client().add_ca_to_store(
                    name,
                    ContentAddressMethodAlgorithm::Text,
                    &Default::default(),
                    false,
                    source,
                ))
                .await
            }
        })?;
        Ok(info.path)
    }

    /// NAR a filesystem path then upload it as a recursive-CA (NAR-hashed)
    /// store object.
    pub fn add_to_store_nar(&self, name: &str, path: &Path) -> Result<StorePath> {
        let nar_bytes = encode_nar(path)?;
        let info = self.runtime.block_on(async {
            let mut guard = self.pool.acquire().await?;
            let source = BufReader::new(Cursor::new(nar_bytes));
            if self.in_drv {
                await_result(guard.client().add_to_store_scanning(
                    name,
                    ContentAddressMethodAlgorithm::Recursive(
                        harmonia_utils_hash::Algorithm::SHA256,
                    ),
                    source,
                ))
                .await
            } else {
                // Suboptimal fallback, see add_to_store_text
                await_result(guard.client().add_ca_to_store(
                    name,
                    ContentAddressMethodAlgorithm::Recursive(
                        harmonia_utils_hash::Algorithm::SHA256,
                    ),
                    &Default::default(),
                    false,
                    source,
                ))
                .await
            }
        })?;
        Ok(info.path)
    }

    /// Declare `path` as the named output of the currently-running
    /// derivation. The path's name must equal
    /// `outputPathName(callingDrv.name, name)`.
    pub fn submit_output(&self, path: &SingleDerivedPath, name: &OutputName) -> Result<()> {
        self.runtime.block_on(async {
            let mut guard = self.pool.acquire().await?;
            await_result(guard.client().submit_output(path, name)).await
        })?;
        Ok(())
    }
}

/// `$NIX_REMOTE` is typically `unix:///abs/path/to/socket` or the legacy
/// alias `daemon`/`auto` that means "default socket". Anything else (e.g.
/// `https://...`, `s3://...`) is unsupported here.
fn parse_unix_remote(remote: &str) -> Result<PathBuf> {
    if matches!(remote, "daemon" | "auto" | "") {
        return Ok(PathBuf::from(DEFAULT_DAEMON_SOCKET));
    }
    if let Some(stripped) = remote.strip_prefix("unix://") {
        return Ok(PathBuf::from(stripped));
    }
    if remote.starts_with('/') {
        return Ok(PathBuf::from(remote));
    }
    Err(Error::UnsupportedRemote(remote.to_string()))
}

/// Drive a `ResultLog` to completion, draining log messages while the future
/// resolves. Daemon-emitted log messages are dropped; we don't currently
/// surface them to nix-ninja.
async fn await_result<T, RL>(rl: RL) -> std::result::Result<T, DaemonError>
where
    RL: futures::Stream<Item = harmonia_protocol::log::LogMessage>
        + futures::Future<Output = std::result::Result<T, DaemonError>>
        + Send,
{
    let mut rl = Box::pin(rl);
    while let Some(_msg) = rl.as_mut().next().await {
        // discard logs for now
    }
    rl.await
}

fn encode_nar(path: &Path) -> Result<Vec<u8>> {
    let mut encoder = nix_nar::Encoder::new(path).map_err(|e| Error::Nar(e.to_string()))?;
    let mut buf = Vec::new();
    std::io::copy(&mut encoder, &mut buf)?;
    Ok(buf)
}
