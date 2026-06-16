//! 会话管理、认证状态机、生命周期
//! M1 实现：连接级认证状态机、argon2 密码校验、按 IP 的暴力破解防护。

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, SaltString};
use argon2::{Argon2, PasswordVerifier};
use tracing::warn;
use uuid::Uuid;

/// 连接级认证状态机：`Unauthenticated → Authenticated`。
#[derive(Debug, Clone)]
pub enum AuthState {
    /// 未认证，仅接受 auth/ping 消息。
    Unauthenticated,
    /// 已认证，附带会话 ID。
    Authenticated { session_id: Uuid },
}

/// 一个已认证连接的会话信息。连接断开即销毁。
#[derive(Debug, Clone)]
pub struct Session {
    pub id: Uuid,
    pub peer_addr: SocketAddr,
    pub authenticated_at: Instant,
    pub last_active: Instant,
}

impl Session {
    /// 为指定连接新建会话。
    pub fn new(peer_addr: SocketAddr) -> Self {
        let now = Instant::now();
        Self {
            id: Uuid::new_v4(),
            peer_addr,
            authenticated_at: now,
            last_active: now,
        }
    }

    /// 更新最近活跃时间（用于空闲超时判定）。
    pub fn touch(&mut self) {
        self.last_active = Instant::now();
    }
}

/// 密码校验器：内存中保存 argon2 哈希，校验时常数时间比较。
pub struct Authenticator {
    /// argon2 PHC 格式哈希字符串。
    password_hash: String,
}

impl Authenticator {
    /// 用明文密码构造：在内存中生成 argon2 哈希，不保留明文。
    pub fn from_password(password: &str) -> anyhow::Result<Self> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| anyhow::anyhow!("生成密码哈希失败: {e}"))?
            .to_string();
        Ok(Self {
            password_hash: hash,
        })
    }

    /// 直接用配置中给定的 argon2 PHC 哈希字符串构造（优先于明文）。
    pub fn from_hash(hash: &str) -> anyhow::Result<Self> {
        // 提前校验哈希格式，避免运行期每次校验才发现配置错误。
        PasswordHash::new(hash).map_err(|e| anyhow::anyhow!("无效的 argon2 密码哈希: {e}"))?;
        Ok(Self {
            password_hash: hash.to_string(),
        })
    }

    /// 校验 token 是否匹配密码。argon2 校验本身为常数时间。
    pub fn verify(&self, token: &str) -> bool {
        let parsed = match PasswordHash::new(&self.password_hash) {
            Ok(h) => h,
            Err(e) => {
                warn!("密码哈希解析失败: {e}");
                return false;
            }
        };
        Argon2::default()
            .verify_password(token.as_bytes(), &parsed)
            .is_ok()
    }
}

/// 单个 IP 的失败记录。
#[derive(Debug, Clone)]
struct FailRecord {
    /// 累计失败次数。
    failures: u32,
    /// 封禁截止时间（None 表示未封禁）。
    banned_until: Option<Instant>,
}

/// 暴力破解防护：按来源 IP 记录失败次数并实施封禁。
pub struct BruteForceGuard {
    records: Mutex<HashMap<IpAddr, FailRecord>>,
    /// 达到该失败次数后封禁。
    max_failures: u32,
    /// 封禁时长。
    ban_duration: Duration,
}

impl BruteForceGuard {
    /// 用安全配置参数构造。
    pub fn new(max_failures: u32, ban_seconds: u64) -> Self {
        Self {
            records: Mutex::new(HashMap::new()),
            max_failures,
            ban_duration: Duration::from_secs(ban_seconds),
        }
    }

    /// 查询该 IP 是否处于封禁中，返回剩余封禁时长。
    pub fn is_banned(&self, ip: IpAddr) -> Option<Duration> {
        let mut records = self.records.lock().unwrap();
        let record = records.get(&ip)?;
        let until = record.banned_until?;
        let now = Instant::now();
        if until > now {
            Some(until - now)
        } else {
            // 封禁已过期，清除记录，允许重新尝试。
            records.remove(&ip);
            None
        }
    }

    /// 记录一次失败：计数 +1，达到阈值后封禁。
    pub fn record_failure(&self, ip: IpAddr) {
        let mut records = self.records.lock().unwrap();
        let record = records.entry(ip).or_insert(FailRecord {
            failures: 0,
            banned_until: None,
        });
        record.failures += 1;
        if record.failures >= self.max_failures {
            record.banned_until = Some(Instant::now() + self.ban_duration);
        }
    }

    /// 认证成功后清除该 IP 的失败记录。
    pub fn record_success(&self, ip: IpAddr) {
        self.records.lock().unwrap().remove(&ip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn test_ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, last))
    }

    #[test]
    fn authenticator_accepts_correct_and_rejects_wrong_password() {
        let auth = Authenticator::from_password("correct-horse").unwrap();
        assert!(auth.verify("correct-horse"));
        assert!(!auth.verify("wrong-password"));
        assert!(!auth.verify(""));
    }

    #[test]
    fn authenticator_from_hash_roundtrip() {
        // 先用明文生成哈希，再用 from_hash 复用该哈希。
        let original = Authenticator::from_password("p@ss").unwrap();
        let rebuilt = Authenticator::from_hash(&original.password_hash).unwrap();
        assert!(rebuilt.verify("p@ss"));
        assert!(!rebuilt.verify("nope"));
    }

    #[test]
    fn authenticator_from_hash_rejects_invalid() {
        assert!(Authenticator::from_hash("not-a-valid-phc-hash").is_err());
    }

    #[test]
    fn guard_counts_failures_and_bans() {
        let guard = BruteForceGuard::new(3, 300);
        let ip = test_ip(1);

        assert!(guard.is_banned(ip).is_none());
        guard.record_failure(ip);
        guard.record_failure(ip);
        // 还未达到阈值。
        assert!(guard.is_banned(ip).is_none());
        guard.record_failure(ip);
        // 第 3 次失败触发封禁。
        let remaining = guard.is_banned(ip).expect("应已封禁");
        assert!(remaining <= Duration::from_secs(300) && remaining > Duration::ZERO);
    }

    #[test]
    fn guard_success_clears_record() {
        let guard = BruteForceGuard::new(3, 300);
        let ip = test_ip(2);
        guard.record_failure(ip);
        guard.record_failure(ip);
        guard.record_success(ip);
        // 清除后重新计数，单次失败不应封禁。
        guard.record_failure(ip);
        assert!(guard.is_banned(ip).is_none());
    }

    #[test]
    fn guard_ban_expires() {
        // 封禁 0 秒，应立即过期。
        let guard = BruteForceGuard::new(1, 0);
        let ip = test_ip(3);
        guard.record_failure(ip);
        assert!(guard.is_banned(ip).is_none());
    }

    #[test]
    fn guard_isolates_per_ip() {
        let guard = BruteForceGuard::new(2, 300);
        let ip_a = test_ip(10);
        let ip_b = test_ip(11);
        guard.record_failure(ip_a);
        guard.record_failure(ip_a);
        assert!(guard.is_banned(ip_a).is_some());
        // 另一个 IP 不受影响。
        assert!(guard.is_banned(ip_b).is_none());
    }
}
