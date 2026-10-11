use borsh::BorshDeserialize;
use oi_types::traits::StakeRegistry;
use oi_types::{Error, Result, StakeInfo};
use solana_client::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;

const STAKE_SEED: &[u8] = b"stake";
const REPUTATION_ROOT_SEED: &[u8] = b"reputation_root";

/// Mirrors the on-chain StakeAccount layout (legacy V1) for client-side deserialization.
/// Must stay in sync with programs/opensonarx-staking/src/state.rs.
#[derive(BorshDeserialize, Debug)]
struct StakeAccount {
    is_initialized: bool,
    staker: [u8; 32],
    domain: String,
    amount_lamports: u64,
    staked_at: i64,
    cooldown_until: i64,
}

/// Mirrors the on-chain StakeAccountV2 layout. The V2 reader below is not
/// wired to a caller yet; only V1 bonds are read today.
#[allow(dead_code)]
#[derive(BorshDeserialize, Debug)]
struct StakeAccountV2 {
    is_initialized: bool,
    staker: [u8; 32],
    domain: String,
    amount: u64,
    staked_at: i64,
    cooldown_until: i64,
    stake_type: u8,
    reward_debt: u128,
    dns_verified: bool,
    slashed_amount: u64,
    _bump: u8,
}

/// StakeRegistry backed by on-chain Solana program data via RPC.
pub struct SolanaStakeRegistry {
    rpc: RpcClient,
    program_id: Pubkey,
}

impl SolanaStakeRegistry {
    pub fn new(rpc_url: &str, program_id: Pubkey) -> Self {
        Self {
            // `confirmed`, not the default `finalized`: a bond should be
            // readable as soon as the cluster has confirmed it, not some
            // thirty slots later.
            rpc: RpcClient::new_with_commitment(
                rpc_url.to_string(),
                solana_sdk::commitment_config::CommitmentConfig::confirmed(),
            ),
            program_id,
        }
    }

    fn derive_pda(&self, domain: &str) -> Pubkey {
        let (pda, _) =
            Pubkey::find_program_address(&[STAKE_SEED, domain.as_bytes()], &self.program_id);
        pda
    }

    #[allow(dead_code)]
    fn derive_pda_v2(&self, domain: &str, staker: &Pubkey) -> Pubkey {
        let (pda, _) = Pubkey::find_program_address(
            &[STAKE_SEED, domain.as_bytes(), staker.as_ref()],
            &self.program_id,
        );
        pda
    }

    // Program accounts are allocated at their maximum size and written from
    // the start, so the data ends in zero padding. `deserialize` reads the
    // value and ignores the padding; `try_from_slice` would reject it.
    fn read_account(&self, domain: &str) -> Result<Option<StakeAccount>> {
        let pda = self.derive_pda(domain);
        match self.rpc.get_account_data(&pda) {
            Ok(data) => {
                let account = StakeAccount::deserialize(&mut &data[..])
                    .map_err(|e| Error::Internal(format!("borsh deserialize: {e}")))?;
                if account.is_initialized {
                    Ok(Some(account))
                } else {
                    Ok(None)
                }
            }
            Err(_) => Ok(None),
        }
    }

    #[allow(dead_code)]
    fn read_account_v2(&self, domain: &str, staker: &Pubkey) -> Result<Option<StakeAccountV2>> {
        let pda = self.derive_pda_v2(domain, staker);
        match self.rpc.get_account_data(&pda) {
            Ok(data) => {
                let account = StakeAccountV2::deserialize(&mut &data[..])
                    .map_err(|e| Error::Internal(format!("borsh deserialize v2: {e}")))?;
                if account.is_initialized {
                    Ok(Some(account))
                } else {
                    Ok(None)
                }
            }
            Err(_) => Ok(None),
        }
    }

    fn account_to_info(account: StakeAccount) -> StakeInfo {
        let cooldown = if account.cooldown_until > 0 {
            Some(account.cooldown_until)
        } else {
            None
        };
        StakeInfo {
            domain: account.domain,
            amount_lamports: account.amount_lamports,
            amount_usd: account.amount_lamports as f64 / 1_000_000_000.0 * 150.0,
            staked_at: account.staked_at,
            cooldown_until: cooldown,
            wallet: Pubkey::new_from_array(account.staker).to_string(),
            dns_verified: true,
            reputation: 1.0,
            entity: None,
            disputed: false,
        }
    }

    #[allow(dead_code)]
    fn account_v2_to_info(account: StakeAccountV2) -> StakeInfo {
        let cooldown = if account.cooldown_until > 0 {
            Some(account.cooldown_until)
        } else {
            None
        };
        StakeInfo {
            domain: account.domain,
            amount_lamports: account.amount,
            amount_usd: account.amount as f64 / 1_000_000_000.0, // $TRUTH tokens, 1:1 for now
            staked_at: account.staked_at,
            cooldown_until: cooldown,
            wallet: Pubkey::new_from_array(account.staker).to_string(),
            dns_verified: account.dns_verified,
            reputation: 1.0,
            entity: None,
            disputed: false,
        }
    }
}

/// Mirrors the on-chain ReputationRootAccount layout for client-side deserialization.
#[derive(BorshDeserialize, Debug)]
struct ReputationRootAccount {
    is_initialized: bool,
    merkle_root: [u8; 32],
    epoch: u64,
    _timestamp: i64,
    _bump: u8,
}

impl SolanaStakeRegistry {
    /// Read the on-chain reputation root (if it exists).
    /// Returns `Ok(Some((merkle_root, epoch)))` or `Ok(None)` if not initialized.
    pub fn get_reputation_root(&self) -> Result<Option<([u8; 32], u64)>> {
        let (pda, _) = Pubkey::find_program_address(&[REPUTATION_ROOT_SEED], &self.program_id);
        match self.rpc.get_account_data(&pda) {
            Ok(data) => {
                let account = ReputationRootAccount::deserialize(&mut &data[..]).map_err(|e| {
                    Error::Internal(format!("borsh deserialize reputation root: {e}"))
                })?;
                if account.is_initialized {
                    Ok(Some((account.merkle_root, account.epoch)))
                } else {
                    Ok(None)
                }
            }
            Err(_) => Ok(None),
        }
    }
}

impl StakeRegistry for SolanaStakeRegistry {
    fn get_stake(&self, domain: &str) -> Result<Option<StakeInfo>> {
        Ok(self.read_account(domain)?.map(Self::account_to_info))
    }

    fn list_active_stakes(&self) -> Result<Vec<StakeInfo>> {
        // Full implementation would use getProgramAccounts RPC.
        Ok(vec![])
    }

    fn is_staked(&self, domain: &str) -> Result<bool> {
        Ok(self
            .read_account(domain)?
            .map(|a| a.cooldown_until == 0)
            .unwrap_or(false))
    }
}
