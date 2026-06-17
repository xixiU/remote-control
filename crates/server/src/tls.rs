//! TLS 配置模块（M4）
//!
//! 支持三种模式：
//! 1. 用户提供证书/私钥路径 → 直接加载
//! 2. 缓存的自签名证书存在 → 从磁盘加载
//! 3. 首次运行 → 用 rcgen 生成自签名证书并缓存到 .cache/

use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use rcgen::{CertificateParams, KeyPair, SanType};
use rustls::pki_types::CertificateDer;
use rustls::ServerConfig;
use tracing::{info, warn};

use crate::config::TlsConfig;

/// 默认缓存目录（相对于 CWD）
const CACHE_DIR: &str = ".cache";
const CACHED_CERT: &str = ".cache/cert.pem";
const CACHED_KEY: &str = ".cache/key.pem";

/// 根据 TlsConfig 构建 rustls ServerConfig（返回 Arc 供 axum-server 使用）。
pub fn make_rustls_config(tls_cfg: &TlsConfig) -> Result<Arc<ServerConfig>> {
    let (cert_pem, key_pem) = load_or_generate(tls_cfg)?;

    let certs = rustls_pemfile::certs(&mut BufReader::new(cert_pem.as_bytes()))
        .collect::<Result<Vec<CertificateDer<'_>>, _>>()
        .context("解析证书 PEM 失败")?;

    let key = rustls_pemfile::private_key(&mut BufReader::new(key_pem.as_bytes()))
        .context("解析私钥 PEM 失败")?
        .context("私钥文件中未找到有效私钥")?;

    let config = ServerConfig::builder_with_provider(
        std::sync::Arc::new(rustls::crypto::ring::default_provider()),
    )
    .with_safe_default_protocol_versions()
    .context("协商 TLS 协议版本失败")?
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .context("构建 TLS ServerConfig 失败")?;

    Ok(Arc::new(config))
}

/// 加载或生成证书/私钥 PEM 字符串。
fn load_or_generate(tls_cfg: &TlsConfig) -> Result<(String, String)> {
    // 模式 1：用户指定了证书和私钥路径
    if let (Some(cert_path), Some(key_path)) = (&tls_cfg.cert, &tls_cfg.key) {
        info!("使用用户指定的证书: {}, 私钥: {}", cert_path, key_path);
        let cert_pem = fs::read_to_string(cert_path)
            .with_context(|| format!("读取证书文件失败: {}", cert_path))?;
        let key_pem = fs::read_to_string(key_path)
            .with_context(|| format!("读取私钥文件失败: {}", key_path))?;
        return Ok((cert_pem, key_pem));
    }

    // 模式 2：尝试从缓存加载
    let cert_cache = PathBuf::from(CACHED_CERT);
    let key_cache = PathBuf::from(CACHED_KEY);

    if cert_cache.exists() && key_cache.exists() {
        info!("从缓存加载自签名证书: {}", cert_cache.display());
        let cert_pem = fs::read_to_string(&cert_cache)
            .context("读取缓存证书失败")?;
        let key_pem = fs::read_to_string(&key_cache)
            .context("读取缓存私钥失败")?;
        return Ok((cert_pem, key_pem));
    }

    // 模式 3：生成自签名证书并缓存
    info!("生成自签名证书（CN=localhost, SANs: localhost, 127.0.0.1）");
    let (cert_pem, key_pem) = generate_self_signed()?;

    // 写入缓存
    if let Err(e) = cache_to_disk(&cert_pem, &key_pem) {
        warn!("缓存证书到磁盘失败（下次启动将重新生成）: {}", e);
    }

    Ok((cert_pem, key_pem))
}

/// 用 rcgen 生成自签名证书。
fn generate_self_signed() -> Result<(String, String)> {
    let mut params = CertificateParams::new(vec!["localhost".to_string()])
        .context("创建证书参数失败")?;

    // 添加 SAN: IP 127.0.0.1
    params.subject_alt_names.push(SanType::IpAddress(
        std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)),
    ));

    let key_pair = KeyPair::generate().context("生成密钥对失败")?;
    let cert = params.self_signed(&key_pair).context("生成自签名证书失败")?;

    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    Ok((cert_pem, key_pem))
}

/// 将证书和私钥写入缓存目录。
fn cache_to_disk(cert_pem: &str, key_pem: &str) -> Result<()> {
    let cache_dir = Path::new(CACHE_DIR);
    if !cache_dir.exists() {
        fs::create_dir_all(cache_dir).context("创建 .cache 目录失败")?;
    }

    fs::write(CACHED_CERT, cert_pem).context("写入证书缓存失败")?;
    fs::write(CACHED_KEY, key_pem).context("写入私钥缓存失败")?;

    info!("自签名证书已缓存到 {}", cache_dir.display());
    Ok(())
}
