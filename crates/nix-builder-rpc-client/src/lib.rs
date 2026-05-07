//! Sync client for the Nix daemon's worker protocol, backed by
//! `harmonia_store_remote::pool::ConnectionPool` for concurrent acquisition
//! across worker threads. Inside a `builder-rpc-v0` sandbox
//! (NixOS/nix#15793), the daemon socket is exposed via `$NIX_REMOTE` and
//! only a small allowlist is permitted: `Add{ToStore,ToStoreNar,TextToStore}`
//! plus `SubmitOutput`. Outside the sandbox the same client talks to the
//! standard daemon socket.

pub mod aterm;

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use harmonia_protocol::daemon_wire::types2::{BuildMode, BuildResultInner};
use harmonia_protocol::store_path::StoreDir;
use harmonia_protocol::types::{DaemonError, DaemonStore};
use harmonia_store_content_address::ContentAddressMethodAlgorithm;
use harmonia_store_derivation::derivation::Derivation;
use harmonia_store_derivation::derived_path::{
    DerivedPath, OutputName, OutputSpec, SingleDerivedPath,
};
use harmonia_store_path::{StorePath, StorePathSet};
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
    #[error("No Nix daemon running, if you are on a single-user Nix install run `nix-daemon`")]
    NoDaemon,
    #[error("nar encode: {0}")]
    Nar(String),
    #[error("build of {path} failed: {error_msg}")]
    BuildFailed { path: String, error_msg: String },
    #[error("daemon returned no build result for {0}")]
    MissingBuildResult(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct BuilderRpcClient {
    /// Multi-thread so the pool's RAII drop tasks can complete asynchronously.
    runtime: Runtime,
    pool: ConnectionPool,
    /// Whether or not nix-ninja is running within a derivation
    /// Needed since scanning is not available outside derivations
    in_drv: bool,

    /// ATerm bytes kept because builder-rpc-v0 does not materialize uploaded .drv files in the sandbox.
    uploaded_drvs: Mutex<HashMap<StorePath, Vec<u8>>>,
}

impl BuilderRpcClient {
    /// Connect to `$NIX_REMOTE` if set, otherwise the standard daemon
    pub fn connect_from_env() -> Result<Self> {
        let path = match std::env::var(SOCKET_ENV) {
            Ok(remote) => parse_unix_remote(&remote)?,
            Err(_) => PathBuf::from(DEFAULT_DAEMON_SOCKET),
        };
        if !path.exists() {
            return Err(Error::NoDaemon);
        }

        // There is no simple OS flag to tell if we are running in a derivation,
        // but NIX_BUILD_TOP is always set by the nix daemon before building
        // and has no purpose outside a nix derivation.
        let in_drv = std::env::var_os("NIX_BUILD_TOP").is_some();

        Self::connect_unix(&path, in_drv)
    }

    pub fn connect_unix(path: &Path, in_drv: bool) -> Result<Self> {
        let runtime = RuntimeBuilder::new_multi_thread().enable_all().build()?;
        let pool = ConnectionPool::new(path, PoolConfig::default());
        Ok(Self {
            runtime,
            pool,
            in_drv,
            uploaded_drvs: Default::default(),
        })
    }

    pub fn clone_drv(&self, store_path: &StorePath) -> Option<Vec<u8>> {
        self.uploaded_drvs.lock().unwrap().get(store_path).cloned()
    }

    // Serialise a derivation and add it to the store.
    // Should be preferred to `add_to_store_text` for derivations,
    pub fn add_drv_to_store(&self, store_dir: &StoreDir, drv: &Derivation) -> Result<StorePath> {
        let bytes = aterm::print_derivation_aterm(store_dir, drv);
        let refs: StorePathSet = drv.inputs.iter().map(|p| p.root_path().clone()).collect();
        let name = format!("{}.drv", drv.name);
        let info = self.runtime.block_on(async {
            let mut guard = self.pool.acquire().await?;
            let source = BufReader::new(Cursor::new(bytes.clone()));
            guard
                .execute(|client| {
                    client.add_ca_to_store(
                        &name,
                        ContentAddressMethodAlgorithm::Text,
                        &refs,
                        false,
                        source,
                    )
                })
                .await
        })?;

        self.uploaded_drvs
            .lock()
            .unwrap()
            .insert(info.path.clone(), bytes);
        Ok(info.path)
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
                guard
                    .execute(|client| {
                        client.add_to_store_scanning(
                            name,
                            ContentAddressMethodAlgorithm::Text,
                            source,
                        )
                    })
                    .await
            } else {
                // Unfortunately outside of a derivation we do not have a good
                // idea of possible paths for which we can scan.
                // A trivial "scan for all paths" implementation would include nonexistent paths
                // from documentation and fail on important projects, e.g. nix.
                // An empty reference set is identical to the fallback `nix store add` case
                // and preferable to no running outside a derivation at all.
                let refs = Default::default();
                guard
                    .execute(|client| {
                        client.add_ca_to_store(
                            name,
                            ContentAddressMethodAlgorithm::Text,
                            &refs,
                            false,
                            source,
                        )
                    })
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
                guard
                    .execute(|client| {
                        client.add_to_store_scanning(
                            name,
                            ContentAddressMethodAlgorithm::NixArchive(
                                harmonia_utils_hash::Algorithm::SHA256,
                            ),
                            source,
                        )
                    })
                    .await
            } else {
                // Suboptimal fallback, see add_to_store_text
                let refs = Default::default();
                guard
                    .execute(|client| {
                        client.add_ca_to_store(
                            name,
                            ContentAddressMethodAlgorithm::NixArchive(
                                harmonia_utils_hash::Algorithm::SHA256,
                            ),
                            &refs,
                            false,
                            source,
                        )
                    })
                    .await
            }
        })?;
        Ok(info.path)
    }

    /// Build the given derived paths and return the store path each one
    /// resolves to, in input order. Only usable outside a `builder-rpc-v0`
    /// sandbox — the restricted allowlist does not include `BuildPaths`.
    pub fn build_paths(
        &self,
        store_dir: &StoreDir,
        paths: &[SingleDerivedPath],
    ) -> Result<Vec<StorePath>> {
        let derived: Vec<DerivedPath> = paths
            .iter()
            .map(|p| match p {
                SingleDerivedPath::Opaque(path) => DerivedPath::Opaque(path.clone()),
                SingleDerivedPath::Built { drv_path, output } => DerivedPath::Built {
                    drv_path: drv_path.clone(),
                    outputs: OutputSpec::Named(std::iter::once(output.clone()).collect()),
                },
            })
            .collect();

        let results = self.runtime.block_on(async {
            let mut guard = self.pool.acquire().await?;
            guard
                .execute(|client| client.build_paths_with_results(&derived, BuildMode::Normal))
                .await
        })?;

        let by_path: HashMap<_, _> = results.into_iter().map(|r| (r.path, r.result)).collect();
        paths
            .iter()
            .zip(&derived)
            .map(|(single, derived_path)| {
                let display = store_dir.display(single).to_string();
                let result = by_path
                    .get(derived_path)
                    .ok_or_else(|| Error::MissingBuildResult(display.clone()))?;
                let success = result.success().ok_or_else(|| Error::BuildFailed {
                    path: display.clone(),
                    error_msg: match &result.inner {
                        BuildResultInner::Failure(f) => {
                            String::from_utf8_lossy(&f.error_msg).into_owned()
                        }
                        _ => String::new(),
                    },
                })?;
                match single {
                    SingleDerivedPath::Opaque(path) => Ok(path.clone()),
                    SingleDerivedPath::Built { output, .. } => success
                        .built_outputs
                        .get(output)
                        .map(|realisation| realisation.out_path.clone())
                        .ok_or_else(|| Error::MissingBuildResult(display)),
                }
            })
            .collect()
    }

    /// Declare `path` as the named output of the currently-running
    /// derivation. The path's name must equal
    /// `outputPathName(callingDrv.name, name)`.
    pub fn submit_output(&self, path: &SingleDerivedPath, name: &OutputName) -> Result<()> {
        self.runtime.block_on(async {
            let mut guard = self.pool.acquire().await?;
            guard
                .execute(|client| client.submit_output(path, name))
                .await
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

fn encode_nar(path: &Path) -> Result<Vec<u8>> {
    let mut encoder = nix_nar::Encoder::new(path).map_err(|e| Error::Nar(e.to_string()))?;
    let mut buf = Vec::new();
    std::io::copy(&mut encoder, &mut buf)?;
    Ok(buf)
}
