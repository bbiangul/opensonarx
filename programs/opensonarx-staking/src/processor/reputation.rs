use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    msg,
    program::invoke_signed,
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
    system_instruction,
    sysvar::Sysvar,
};

use crate::error::StakingError;
use crate::processor::{deser, deser_config};
use crate::state::{ReputationRootAccount, CONFIG_SEED, REPUTATION_ROOT_SEED};

/// Process the CommitReputationRoot instruction.
///
/// Accounts:
///   0. `[signer]` Authority (must match config.authority)
///   1. `[writable]` ReputationRootAccount PDA
///   2. `[]` Config PDA
///   3. `[]` System Program
pub fn process_commit_reputation_root(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    merkle_root: [u8; 32],
    epoch: u64,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let authority = next_account_info(accounts_iter)?;
    let reputation_pda = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;
    let system_program = next_account_info(accounts_iter)?;

    // Authority must be a signer
    if !authority.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // Validate config PDA
    let (expected_config, _) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    if *config_pda.key != expected_config {
        return Err(ProgramError::InvalidSeeds);
    }

    // Read config and verify authority (uses version-aware deser for pre-upgrade compat)
    let config = deser_config(&config_pda.data.borrow())?;
    if !config.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }
    if config.authority != *authority.key {
        msg!("Authority mismatch");
        return Err(StakingError::UnauthorizedReputationRoot.into());
    }

    // Validate reputation PDA
    let (expected_pda, bump) =
        Pubkey::find_program_address(&[REPUTATION_ROOT_SEED], program_id);
    if *reputation_pda.key != expected_pda {
        return Err(ProgramError::InvalidSeeds);
    }

    let clock = Clock::get()?;

    // Check if account exists and is initialized
    if reputation_pda.data_len() > 0 && reputation_pda.lamports() > 0 {
        // Account exists — validate epoch is increasing
        if let Ok(existing) = deser::<ReputationRootAccount>(&reputation_pda.data.borrow()) {
            if existing.is_initialized && epoch <= existing.epoch {
                msg!("Epoch {} is not greater than current {}", epoch, existing.epoch);
                return Err(StakingError::StaleEpoch.into());
            }
        }

        // Update existing account
        let account = ReputationRootAccount {
            is_initialized: true,
            merkle_root,
            epoch,
            timestamp: clock.unix_timestamp,
            bump,
        };
        borsh::to_writer(&mut &mut reputation_pda.data.borrow_mut()[..], &account)?;
    } else {
        // Create new PDA account
        let space = ReputationRootAccount::MAX_SIZE;
        let rent = Rent::get()?;
        let lamports = rent.minimum_balance(space);

        invoke_signed(
            &system_instruction::create_account(
                authority.key,
                reputation_pda.key,
                lamports,
                space as u64,
                program_id,
            ),
            &[
                authority.clone(),
                reputation_pda.clone(),
                system_program.clone(),
            ],
            &[&[REPUTATION_ROOT_SEED, &[bump]]],
        )?;

        let account = ReputationRootAccount {
            is_initialized: true,
            merkle_root,
            epoch,
            timestamp: clock.unix_timestamp,
            bump,
        };
        borsh::to_writer(&mut &mut reputation_pda.data.borrow_mut()[..], &account)?;
    }

    msg!("Committed reputation root for epoch {}", epoch);
    Ok(())
}
