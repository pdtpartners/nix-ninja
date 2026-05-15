use anyhow::Result;
use nix_builder_rpc_client::BuilderRpcClient;
use std::sync::{Arc, OnceLock};

/// Lazily-initialized, shareable handle to a `BuilderRpcClient`. The client
/// is backed by `harmonia_store_remote::ConnectionPool`, so worker threads
/// can hit the daemon concurrently with no external locking. `None` after
/// init means no daemon socket exists; caller falls back.
#[derive(Clone, Default)]
pub struct RpcClient {
    inner: Arc<OnceLock<Option<BuilderRpcClient>>>,
}

impl RpcClient {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run `f` with the shared client. Lazily resolves `$NIX_REMOTE`
    /// on first call; passes `None` when no daemon is reachable.
    pub fn with<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(Option<&BuilderRpcClient>) -> Result<R>,
    {
        let slot = match self.inner.get() {
            Some(s) => s,
            None => {
                let new = BuilderRpcClient::connect_from_env()?;
                // Race: if another thread won, our `new` is dropped; both
                // end up reading the same Some/None.
                self.inner.set(new).ok();
                self.inner.get().unwrap()
            }
        };
        f(slot.as_ref())
    }
}
