use super::{deser, deser_config};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    msg,
    program::invoke,
    program::invoke_signed,
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
    system_instruction,
    sysvar::Sysvar,
};

use crate::error::StakingError;
use crate::state::*;

/// Verify that the previous instruction is an Ed25519 precompile instruction
/// and that the signed message matches our expected ticket data.
///
/// The Ed25519 instruction data format (per Solana docs):
///   - byte 0: num_signatures (u8)
///   - byte 1: padding (u8)
///   - For each signature (16 bytes of offset data):
///     - bytes 0-1: signature_offset (u16 LE)
///     - bytes 2-3: signature_instruction_index (u16 LE)
///     - bytes 4-5: public_key_offset (u16 LE)
///     - bytes 6-7: public_key_instruction_index (u16 LE)
///     - bytes 8-9: message_data_offset (u16 LE)
///     - bytes 10-11: message_data_size (u16 LE)
///     - bytes 12-13: message_instruction_index (u16 LE)
///     - bytes 14-15: padding (2 bytes)
fn verify_ed25519_ticket(
    ix_sysvar_info: &AccountInfo,
    expected_payer: &Pubkey,
    expected_msg: &[u8],
) -> Result<(), ProgramError> {
    use solana_program::sysvar::instructions as ix_sysvar;

    let current_ix_index = ix_sysvar::load_current_index_checked(ix_sysvar_info)?;

    // The Ed25519 instruction should be immediately before SettleChannel
    if current_ix_index == 0 {
        return Err(StakingError::InvalidSignature.into());
    }
    let ed25519_ix = ix_sysvar::load_instruction_at_checked(
        (current_ix_index - 1) as usize,
        ix_sysvar_info,
    )?;

    // Verify it's the Ed25519 program
    if ed25519_ix.program_id != solana_program::ed25519_program::id() {
        return Err(StakingError::InvalidSignature.into());
    }

    let ix_data = &ed25519_ix.data;
    if ix_data.len() < 2 {
        return Err(StakingError::InvalidSignature.into());
    }

    let num_sigs = ix_data[0] as usize;
    if num_sigs == 0 {
        return Err(StakingError::InvalidSignature.into());
    }

    // Parse first signature's offsets (16 bytes starting at offset 2)
    if ix_data.len() < 2 + 16 {
        return Err(StakingError::InvalidSignature.into());
    }

    let pubkey_offset = u16::from_le_bytes([ix_data[6], ix_data[7]]) as usize;
    let msg_offset = u16::from_le_bytes([ix_data[10], ix_data[11]]) as usize;
    let msg_size = u16::from_le_bytes([ix_data[12], ix_data[13]]) as usize;

    // Extract pubkey (32 bytes)
    if ix_data.len() < pubkey_offset + 32 {
        return Err(StakingError::InvalidSignature.into());
    }
    let pubkey_bytes = &ix_data[pubkey_offset..pubkey_offset + 32];
    let signer_pubkey = Pubkey::new_from_array(
        pubkey_bytes.try_into().map_err(|_| StakingError::InvalidSignature)?
    );

    // Verify signer is the expected payer
    if signer_pubkey != *expected_payer {
        msg!("Ed25519 signer does not match channel payer");
        return Err(StakingError::InvalidSignature.into());
    }

    // Extract and verify message
    if ix_data.len() < msg_offset + msg_size {
        return Err(StakingError::InvalidSignature.into());
    }
    let signed_msg = &ix_data[msg_offset..msg_offset + msg_size];
    if signed_msg != expected_msg {
        msg!("Ed25519 signed message does not match ticket data");
        return Err(StakingError::InvalidSignature.into());
    }

    Ok(())
}

/// OpenChannel: payer deposits $TRUTH into a state channel.
pub fn process_open_channel(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payee_bytes: [u8; 32],
    amount: u64,
    expiry_slot: u64,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let payer = next_account_info(accounts_iter)?;
    let payer_token = next_account_info(accounts_iter)?;
    let vault_token = next_account_info(accounts_iter)?;
    let channel_pda = next_account_info(accounts_iter)?;
    let token_program = next_account_info(accounts_iter)?;
    let system_program = next_account_info(accounts_iter)?;
    let _clock_sysvar = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;

    if !payer.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // Validate config, vault, and token program
    let (expected_config, _) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);
    if *config_pda.key != expected_config {
        return Err(ProgramError::InvalidSeeds);
    }
    let config = deser_config(&config_pda.data.borrow())?;
    if *vault_token.key != config.reward_vault {
        return Err(ProgramError::InvalidSeeds);
    }
    if *token_program.key != spl_token::id() {
        return Err(ProgramError::IncorrectProgramId);
    }

    let payee = Pubkey::new_from_array(payee_bytes);

    // Derive channel PDA: ["channel", payer, payee]
    let (expected_channel, channel_bump) = Pubkey::find_program_address(
        &[CHANNEL_SEED, payer.key.as_ref(), payee.as_ref()],
        program_id,
    );
    if *channel_pda.key != expected_channel {
        return Err(ProgramError::InvalidSeeds);
    }

    // Transfer $TRUTH from payer to vault
    let transfer_ix = spl_token::instruction::transfer(
        token_program.key,
        payer_token.key,
        vault_token.key,
        payer.key,
        &[],
        amount,
    )?;
    invoke(
        &transfer_ix,
        &[payer_token.clone(), vault_token.clone(), payer.clone(), token_program.clone()],
    )?;

    // Create channel account
    let rent = Rent::get()?;
    let space = StateChannelAccount::MAX_SIZE;
    let lamports = rent.minimum_balance(space);
    invoke_signed(
        &system_instruction::create_account(
            payer.key,
            channel_pda.key,
            lamports,
            space as u64,
            program_id,
        ),
        &[payer.clone(), channel_pda.clone(), system_program.clone()],
        &[&[CHANNEL_SEED, payer.key.as_ref(), payee.as_ref(), &[channel_bump]]],
    )?;

    let channel = StateChannelAccount {
        is_initialized: true,
        payer: *payer.key,
        payee,
        deposited: amount,
        spent: 0,
        nonce: 0,
        expiry_slot,
        bump: channel_bump,
    };
    borsh::to_writer(&mut &mut channel_pda.data.borrow_mut()[..], &channel)?;

    msg!("Channel opened: {} → {} for {} $TRUTH", payer.key, payee, amount);
    Ok(())
}

/// SettleChannel: submit highest-nonce ticket, apply governable fee split
/// (burn_pct% burned, sentinel_fee_pct% to sentinels, remainder to payee),
/// refund unspent deposit.
///
/// Accounts:
///   0. `[signer]` Submitter (payer or payee)
///   1. `[writable]` StateChannelAccount PDA
///   2. `[writable]` Payee's $TRUTH token account (serving node — receives its share)
///   3. `[writable]` Payer's $TRUTH token account (refund)
///   4. `[writable]` Program vault token account
///   5. `[]` Config PDA
///   6. `[]` Token Program
///   7. `[writable]` $TRUTH Mint (required — used for burn)
pub fn process_settle_channel(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payer_bytes: [u8; 32],
    payee_bytes: [u8; 32],
    spent: u64,
    nonce: u64,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let submitter = next_account_info(accounts_iter)?;
    let channel_pda = next_account_info(accounts_iter)?;
    let payee_token = next_account_info(accounts_iter)?;
    let payer_token = next_account_info(accounts_iter)?;
    let vault_token = next_account_info(accounts_iter)?;
    let config_pda = next_account_info(accounts_iter)?;
    let token_program = next_account_info(accounts_iter)?;
    let truth_mint = next_account_info(accounts_iter)?;
    // Required: Instructions sysvar for Ed25519 signature verification
    let ix_sysvar = next_account_info(accounts_iter)?;

    if !submitter.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let payer_key = Pubkey::new_from_array(payer_bytes);
    let payee_key = Pubkey::new_from_array(payee_bytes);

    // Only payer or payee can settle
    if *submitter.key != payer_key && *submitter.key != payee_key {
        return Err(StakingError::Unauthorized.into());
    }

    // Verify payer's Ed25519 signature on the ticket (channel_pda + spent + nonce).
    if *ix_sysvar.key != solana_program::sysvar::instructions::id() {
        return Err(ProgramError::InvalidAccountData);
    }
    let expected_msg = [
        channel_pda.key.as_ref(),
        &spent.to_le_bytes(),
        &nonce.to_le_bytes(),
    ].concat();
    verify_ed25519_ticket(ix_sysvar, &payer_key, &expected_msg)?;

    // Derive channel PDA
    let (expected_channel, _) = Pubkey::find_program_address(
        &[CHANNEL_SEED, payer_key.as_ref(), payee_key.as_ref()],
        program_id,
    );
    if *channel_pda.key != expected_channel {
        return Err(ProgramError::InvalidSeeds);
    }

    let channel = deser::<StateChannelAccount>(&channel_pda.data.borrow())?;
    if !channel.is_initialized {
        return Err(ProgramError::UninitializedAccount);
    }

    // Nonce must be strictly higher than stored
    if nonce <= channel.nonce {
        return Err(StakingError::InvalidNonce.into());
    }

    // Check channel expiry — only payer can settle after expiry
    let clock = solana_program::clock::Clock::get()?;
    if clock.slot > channel.expiry_slot && *submitter.key != payer_key {
        return Err(StakingError::ChannelExpired.into());
    }

    // Spent must not exceed deposited
    let actual_spent = std::cmp::min(spent, channel.deposited);
    let refund = channel.deposited.saturating_sub(actual_spent);

    let (_, config_bump) = Pubkey::find_program_address(&[CONFIG_SEED], program_id);

    // Validate vault and token program
    let mut config = deser_config(&config_pda.data.borrow())?;
    if *vault_token.key != config.reward_vault {
        return Err(ProgramError::InvalidSeeds);
    }
    if *token_program.key != spl_token::id() {
        return Err(ProgramError::IncorrectProgramId);
    }

    // ── Fee split (governable burn_pct + sentinel_fee_pct, remainder to payee) ──
    // M1 fix: sentinel fees are accumulated on-chain in the vault, not routed off-chain.
    let burn_amount = actual_spent * config.burn_pct as u64 / 100;
    let sentinel_fee = actual_spent * config.sentinel_fee_pct as u64 / 100;
    // Remainder from integer division goes to payee to avoid token loss
    let payee_amount = actual_spent - burn_amount - sentinel_fee;

    // Sentinel fee stays in vault; accumulate for distribution via UpdateRewardIndex
    config.accumulated_sentinel_fees = config.accumulated_sentinel_fees
        .checked_add(sentinel_fee)
        .ok_or(StakingError::Overflow)?;

    // Transfer payee's share (remainder after burn + sentinel fee)
    if payee_amount > 0 {
        let transfer_ix = spl_token::instruction::transfer(
            token_program.key,
            vault_token.key,
            payee_token.key,
            config_pda.key,
            &[],
            payee_amount,
        )?;
        invoke_signed(
            &transfer_ix,
            &[vault_token.clone(), payee_token.clone(), config_pda.clone(), token_program.clone()],
            &[&[CONFIG_SEED, &[config_bump]]],
        )?;
    }

    // Burn burn_pct% of spent amount
    if burn_amount > 0 {
        if *truth_mint.key != config.truth_mint {
            return Err(ProgramError::InvalidSeeds);
        }
        let burn_ix = spl_token::instruction::burn(
            token_program.key,
            vault_token.key,
            truth_mint.key,
            config_pda.key,
            &[],
            burn_amount,
        )?;
        invoke_signed(
            &burn_ix,
            &[vault_token.clone(), truth_mint.clone(), config_pda.clone(), token_program.clone()],
            &[&[CONFIG_SEED, &[config_bump]]],
        )?;
        msg!("Burned {} $TRUTH from query fees", burn_amount);
    }

    // Refund remainder to payer
    if refund > 0 {
        let refund_ix = spl_token::instruction::transfer(
            token_program.key,
            vault_token.key,
            payer_token.key,
            config_pda.key,
            &[],
            refund,
        )?;
        invoke_signed(
            &refund_ix,
            &[vault_token.clone(), payer_token.clone(), config_pda.clone(), token_program.clone()],
            &[&[CONFIG_SEED, &[config_bump]]],
        )?;
    }

    // Write config back (accumulated_sentinel_fees updated)
    borsh::to_writer(&mut &mut config_pda.data.borrow_mut()[..], &config)?;

    // Close channel PDA
    let channel_lamports = channel_pda.lamports();
    **channel_pda.lamports.borrow_mut() = 0;
    **submitter.lamports.borrow_mut() = submitter
        .lamports()
        .checked_add(channel_lamports)
        .ok_or(StakingError::Overflow)?;
    let mut data = channel_pda.data.borrow_mut();
    for byte in data.iter_mut() {
        *byte = 0;
    }

    msg!(
        "Channel settled: {} to payee, {} burned, {} sentinel fee, {} refund",
        payee_amount, burn_amount, sentinel_fee, refund
    );
    Ok(())
}
