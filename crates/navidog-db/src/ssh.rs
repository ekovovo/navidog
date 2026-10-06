//! SSH 本地端口转发：在 127.0.0.1 的随机端口监听，每个接入的连接开一条 direct-tcpip 通道。

use std::sync::Arc;

use russh::client::{self, Handle};
use russh::keys::{PrivateKeyWithHashAlg, load_secret_key};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::driver::{DbError, DbResult};
use crate::params::SshParams;

struct AcceptAll;

impl client::Handler for AcceptAll {
    type Error = russh::Error;

    // ponytail: 不校验服务器主机密钥（与 Navicat 行为一致）；后续可加 known_hosts 校验
    async fn check_server_key(
        &mut self,
        _key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// 隧道句柄；丢弃时停止监听并断开 SSH。
pub struct Tunnel {
    pub local_port: u16,
    accept: JoinHandle<()>,
    _session: Arc<Handle<AcceptAll>>,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.accept.abort();
    }
}

fn ssh_err(e: impl std::fmt::Display) -> DbError {
    DbError::msg(format!("SSH: {e}"))
}

pub async fn open(ssh: &SshParams, target_host: String, target_port: u16) -> DbResult<Tunnel> {
    let config = Arc::new(client::Config::default());
    let mut session = client::connect(config, (ssh.host.as_str(), ssh.port), AcceptAll)
        .await
        .map_err(ssh_err)?;

    let mut authed = false;
    if let Some(key_path) = &ssh.private_key {
        let key = load_secret_key(key_path, ssh.passphrase.as_deref()).map_err(ssh_err)?;
        let hash = session
            .best_supported_rsa_hash()
            .await
            .map_err(ssh_err)?
            .flatten();
        let res = session
            .authenticate_publickey(&ssh.user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
            .await
            .map_err(ssh_err)?;
        authed = res.success();
    }
    if !authed && let Some(password) = &ssh.password {
        authed = session
            .authenticate_password(&ssh.user, password)
            .await
            .map_err(ssh_err)?
            .success();
    }
    if !authed {
        return Err(DbError::msg("SSH: 身份验证失败"));
    }

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(ssh_err)?;
    let local_port = listener.local_addr().map_err(ssh_err)?.port();
    let session = Arc::new(session);
    let handle = session.clone();
    let accept = tokio::spawn(async move {
        while let Ok((mut socket, peer)) = listener.accept().await {
            let handle = handle.clone();
            let host = target_host.clone();
            tokio::spawn(async move {
                let channel = match handle
                    .channel_open_direct_tcpip(
                        host,
                        target_port as u32,
                        peer.ip().to_string(),
                        peer.port() as u32,
                    )
                    .await
                {
                    Ok(ch) => ch,
                    Err(err) => {
                        tracing::warn!("SSH 通道打开失败: {err}");
                        return;
                    }
                };
                let mut stream = channel.into_stream();
                let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
            });
        }
    });
    Ok(Tunnel {
        local_port,
        accept,
        _session: session,
    })
}
