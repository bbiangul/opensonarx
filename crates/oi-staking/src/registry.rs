use oi_types::{Result, StakeInfo};
use oi_types::traits::StakeRegistry;
use std::collections::HashMap;
use std::sync::RwLock;

/// In-memory stake registry for testing and development
pub struct InMemoryStakeRegistry {
    stakes: RwLock<HashMap<String, StakeInfo>>,
}

impl InMemoryStakeRegistry {
    pub fn new() -> Self {
        Self {
            stakes: RwLock::new(HashMap::new()),
        }
    }

    pub fn add_stake(&self, info: StakeInfo) -> Result<()> {
        let mut stakes = self.stakes.write().unwrap();
        stakes.insert(info.domain.clone(), info);
        Ok(())
    }

    pub fn remove_stake(&self, domain: &str) -> Result<Option<StakeInfo>> {
        let mut stakes = self.stakes.write().unwrap();
        Ok(stakes.remove(domain))
    }
}

impl Default for InMemoryStakeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl StakeRegistry for InMemoryStakeRegistry {
    fn get_stake(&self, domain: &str) -> Result<Option<StakeInfo>> {
        let stakes = self.stakes.read().unwrap();
        Ok(stakes.get(domain).cloned())
    }

    fn list_active_stakes(&self) -> Result<Vec<StakeInfo>> {
        let stakes = self.stakes.read().unwrap();
        Ok(stakes
            .values()
            .filter(|s| s.cooldown_until.is_none())
            .cloned()
            .collect())
    }

    fn is_staked(&self, domain: &str) -> Result<bool> {
        let stakes = self.stakes.read().unwrap();
        match stakes.get(domain) {
            Some(info) => Ok(info.cooldown_until.is_none()),
            None => Ok(false),
        }
    }

    fn get_stakes_batch(&self, domains: &[&str]) -> Result<HashMap<String, StakeInfo>> {
        let stakes = self.stakes.read().unwrap();
        let mut map = HashMap::new();
        for d in domains {
            if let Some(info) = stakes.get(*d) {
                map.insert(d.to_string(), info.clone());
            }
        }
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_stake(domain: &str, amount: u64) -> StakeInfo {
        StakeInfo {
            domain: domain.to_string(),
            amount_lamports: amount,
            amount_usd: amount as f64 / 1_000_000.0,
            staked_at: 1700000000,
            cooldown_until: None,
            wallet: "11111111111111111111111111111111".to_string(),
            dns_verified: true,
            reputation: 1.0,
            entity: None,
            disputed: false,
        }
    }

    #[test]
    fn test_stake_and_get() {
        let reg = InMemoryStakeRegistry::new();
        reg.add_stake(make_stake("example.com", 1_000_000)).unwrap();
        let info = reg.get_stake("example.com").unwrap().unwrap();
        assert_eq!(info.amount_lamports, 1_000_000);
    }

    #[test]
    fn test_is_staked() {
        let reg = InMemoryStakeRegistry::new();
        assert!(!reg.is_staked("example.com").unwrap());
        reg.add_stake(make_stake("example.com", 1_000_000)).unwrap();
        assert!(reg.is_staked("example.com").unwrap());
    }

    #[test]
    fn test_cooldown_not_staked() {
        let reg = InMemoryStakeRegistry::new();
        let mut stake = make_stake("example.com", 1_000_000);
        stake.cooldown_until = Some(1700100000);
        reg.add_stake(stake).unwrap();
        // Domain in cooldown should NOT be considered staked
        assert!(!reg.is_staked("example.com").unwrap());
        // But get_stake still returns it (for info purposes)
        assert!(reg.get_stake("example.com").unwrap().is_some());
    }

    #[test]
    fn test_list_active_stakes() {
        let reg = InMemoryStakeRegistry::new();
        reg.add_stake(make_stake("a.com", 100)).unwrap();
        reg.add_stake(make_stake("b.com", 200)).unwrap();

        // Add one with cooldown
        let mut cooldown = make_stake("c.com", 300);
        cooldown.cooldown_until = Some(1700100000);
        reg.add_stake(cooldown).unwrap();

        let active = reg.list_active_stakes().unwrap();
        assert_eq!(active.len(), 2);
    }

    #[test]
    fn test_remove_stake() {
        let reg = InMemoryStakeRegistry::new();
        reg.add_stake(make_stake("example.com", 1_000_000)).unwrap();
        let removed = reg.remove_stake("example.com").unwrap();
        assert!(removed.is_some());
        assert!(!reg.is_staked("example.com").unwrap());
    }
}
