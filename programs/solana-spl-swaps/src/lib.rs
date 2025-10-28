use anchor_lang::prelude::*;
use anchor_lang::solana_program::hash;
use anchor_spl::token::{self, Mint, Token, TokenAccount};

declare_id!("2WXpY8havGjfRxme9LUxtjFHTh1EfU3ur4v6wiK4KdNC");

/// The size of Anchor's internal discriminator in a PDA's memory
const ANCHOR_DISCRIMINATOR: usize = 8;

#[program]
pub mod solana_spl_swaps {
    use super::*;

    /// Initiates the atomic swap. Funds are transferred from the funder to the token vault.
    /// `swap_amount` represents the quantity of tokens to be transferred through this atomic swap
    /// in base units of the token mint.  
    /// E.g: A quantity of $1 represented by the token "USDC" with "6" decimals
    /// must be provided as 1,000,000.  
    /// `timelock` represents the number of slots after which (non-instant) refunds are allowed.  
    /// `destination_data` can hold optional information regarding the destination chain
    /// in the atomic swap, to be emitted in the logs as-is.
    pub fn initiate(
        ctx: Context<Initiate>,
        redeemer: Pubkey,
        refundee: Pubkey,
        secret_hash: [u8; 32],
        swap_amount: u64, // In base units of the token
        timelock: u64,
        destination_data: Option<Vec<u8>>,
    ) -> Result<()> {
        let Initiate {
            funder,
            funder_token_account,
            mint,
            rent_sponsor,
            token_program,
            token_vault,
            ..
        } = ctx.accounts;

        let token_transfer_context = CpiContext::new(
            token_program.to_account_info(),
            token::Transfer {
                from: funder_token_account.to_account_info(),
                to: token_vault.to_account_info(),
                authority: funder.to_account_info(),
            },
        );
        token::transfer(token_transfer_context, swap_amount)?;

        let expiry_slot = Clock::get()?
            .slot
            .checked_add(timelock)
            .expect("timelock should not cause an overflow");
        *ctx.accounts.swap_data = SwapAccount {
            expiry_slot,
            bump: ctx.bumps.swap_data,
            identity_pda_bump: ctx.bumps.identity_pda,
            rent_sponsor: rent_sponsor.key(),
            mint: mint.key(),
            redeemer,
            refundee,
            secret_hash,
            swap_amount,
            timelock,
        };

        emit!(Initiated {
            timelock,
            mint: mint.key(),
            redeemer,
            refundee,
            secret_hash,
            swap_amount,
            destination_data,
            funder: ctx.accounts.funder.key(),
        });

        Ok(())
    }

    /// Funds are transferred to the redeemer. This instruction does not require any signatures.
    pub fn redeem(ctx: Context<Redeem>, secret: [u8; 32]) -> Result<()> {
        let Redeem {
            identity_pda,
            redeemer_token_account,
            swap_data,
            token_program,
            token_vault,
            ..
        } = ctx.accounts;
        let SwapAccount {
            identity_pda_bump,
            mint,
            redeemer,
            refundee,
            secret_hash,
            swap_amount,
            timelock,
            ..
        } = **swap_data;

        require!(
            hash::hash(&secret).to_bytes() == secret_hash,
            SwapError::InvalidSecret
        );

        let pda_seeds: &[&[&[u8]]] = &[&[&[identity_pda_bump]]];
        let token_transfer_context = CpiContext::new(
            token_program.to_account_info(),
            token::Transfer {
                from: token_vault.to_account_info(),
                to: redeemer_token_account.to_account_info(),
                authority: identity_pda.to_account_info(),
            },
        )
        .with_signer(pda_seeds);
        token::transfer(token_transfer_context, swap_amount)?;

        emit!(Redeemed {
            mint,
            redeemer,
            refundee,
            secret,
            swap_amount,
            timelock,
        });

        Ok(())
    }

    /// Funds are returned to the refundee, given that no redeems have occured
    /// and the expiry slot has been reached.
    /// This instruction does not require any signatures.
    pub fn refund(ctx: Context<Refund>) -> Result<()> {
        let Refund {
            identity_pda,
            refundee_token_account,
            swap_data,
            token_vault,
            token_program,
            ..
        } = ctx.accounts;
        let SwapAccount {
            identity_pda_bump,
            mint,
            redeemer,
            refundee,
            expiry_slot,
            secret_hash,
            swap_amount,
            timelock,
            ..
        } = **swap_data;

        require!(
            Clock::get()?.slot > expiry_slot,
            SwapError::RefundBeforeExpiry
        );

        let pda_seeds: &[&[&[u8]]] = &[&[&[identity_pda_bump]]];
        let token_transfer_context = CpiContext::new(
            token_program.to_account_info(),
            token::Transfer {
                from: token_vault.to_account_info(),
                to: refundee_token_account.to_account_info(),
                authority: identity_pda.to_account_info(),
            },
        )
        .with_signer(pda_seeds);
        token::transfer(token_transfer_context, swap_amount)?;

        emit!(Refunded {
            mint,
            redeemer,
            refundee,
            secret_hash,
            swap_amount,
            timelock,
        });

        Ok(())
    }

    /// Funds are returned to the refundee, with the redeemer's consent.
    /// As such, the redeemer's signature is required for this instruction.
    /// This allows for refunds before the expiry slot.
    pub fn instant_refund(ctx: Context<InstantRefund>) -> Result<()> {
        let InstantRefund {
            identity_pda,
            refundee_token_account,
            swap_data,
            token_program,
            token_vault,
            ..
        } = ctx.accounts;
        let SwapAccount {
            identity_pda_bump,
            mint,
            redeemer,
            refundee,
            secret_hash,
            swap_amount,
            timelock,
            ..
        } = **swap_data;

        let pda_seeds: &[&[&[u8]]] = &[&[&[identity_pda_bump]]];
        let token_transfer_context = CpiContext::new(
            token_program.to_account_info(),
            token::Transfer {
                from: token_vault.to_account_info(),
                to: refundee_token_account.to_account_info(),
                authority: identity_pda.to_account_info(),
            },
        )
        .with_signer(pda_seeds);
        token::transfer(token_transfer_context, swap_amount)?;

        emit!(InstantRefunded {
            mint,
            redeemer,
            refundee,
            secret_hash,
            swap_amount,
            timelock,
        });

        Ok(())
    }
}

/// Stores the state information of the atomic swap on-chain
#[account]
#[derive(InitSpace)]
pub struct SwapAccount {
    /// The bump that derived this PDA.
    /// Storing this makes later verifications less expensive.
    pub bump: u8,
    /// The exact slot after which (non-instant) refunds are allowed
    pub expiry_slot: u64,
    /// The bump associated with the identity pda.
    /// This is needed by the program to authorize token transfers via the token vault.
    pub identity_pda_bump: u8,
    /// The entity that paid the rent fees for the creation of this PDA.
    /// This will be referenced during the refund of the same upon closing this PDA.
    pub rent_sponsor: Pubkey,

    /// The mint for this atomic swap
    pub mint: Pubkey,
    /// The redeemer of the atomic swap
    pub redeemer: Pubkey,
    /// The refundee of the atomic swap
    pub refundee: Pubkey,
    /// The secret hash associated with the atomic swap
    pub secret_hash: [u8; 32],
    /// The quantity tokens to be transferred through this atomic swap
    /// in base units of the token mint.  
    /// E.g: A quantity of $1 represented by the token "USDC" with "6" decimals
    /// must be provided as 1,000,000.
    pub swap_amount: u64,
    /// Represents the number of slots after which (non-instant) refunds are allowed
    pub timelock: u64,
}

#[derive(Accounts)]
// The parameters must have the exact name and order as specified in the underlying function
// to avoid "seed constraint violation" errors.
// Refer: https://www.anchor-lang.com/docs/references/account-constraints#instruction-attribute
#[instruction(redeemer: Pubkey, refundee: Pubkey, secret_hash: [u8; 32], swap_amount: u64, timelock: u64)]
pub struct Initiate<'info> {
    /// CHECK: Program-derived address used solely as signing authority (no data allocation)
    #[account(seeds = [], bump)]
    pub identity_pda: AccountInfo<'info>,

    /// A PDA that maintains the on-chain state of the atomic swap throughout its lifecycle.
    /// The choice of seeds is to make the already expensive possibility of frontrunning, more expensive.
    /// This PDA will be deleted upon completion of the swap.
    #[account(
        init,
        payer = rent_sponsor,
        seeds = [
            mint.key().as_ref(),
            redeemer.as_ref(),
            refundee.as_ref(),
            &secret_hash,
            &swap_amount.to_le_bytes(),
            &timelock.to_le_bytes(),
        ],
        bump,
        space = ANCHOR_DISCRIMINATOR + SwapAccount::INIT_SPACE,
    )]
    pub swap_data: Account<'info, SwapAccount>,

    /// A permanent PDA that is controlled by the program through the `identity_pda`, as implied
    /// by the value of the `authority` field below. As such, it serves as the "vault" by escrowing tokens
    /// of type `mint` for the atomic swap.  
    /// It is intended to be reused for all swaps involving the same mint.  
    /// Just like `identity_pda`, it will be created during the first most invocation of `initiate()`
    /// of every distinct mint using the `init_if_needed` attribute.
    #[account(
        init_if_needed,
        payer = rent_sponsor,
        seeds = [mint.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = identity_pda,
    )]
    pub token_vault: Account<'info, TokenAccount>,

    /// The party that deposits the funds to be involved in the atomic swap.
    /// They must sign this transaction.
    pub funder: Signer<'info>,

    /// The token account of the funder
    #[account(
        mut,
        token::mint = mint,
        token::authority = funder,
    )]
    pub funder_token_account: Account<'info, TokenAccount>,

    /// The mint of the tokens involved in this swap. As this is a parameter, this program can thus be reused
    /// for atomic swaps with different mints.
    pub mint: Account<'info, Mint>,

    /// Any entity that pays the PDA rent.
    /// Upon completion of the swap, the PDA rent refund resulting from the
    /// deletion of `swap_data` will be refunded to this address.
    #[account(mut)]
    pub rent_sponsor: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Redeem<'info> {
    /// CHECK: The Identity PDA, used only for authorizing token transfers, no data is read or written to it
    #[account(seeds = [], bump = swap_data.identity_pda_bump)]
    pub identity_pda: AccountInfo<'info>,

    /// The PDA holding the state information of the atomic swap. Will be closed upon successful execution
    /// and the resulting rent refund will be sent to the rent_sponsor.
    #[account(
        mut,
        seeds = [
            swap_data.mint.as_ref(),
            swap_data.redeemer.as_ref(),
            swap_data.refundee.as_ref(),
            &swap_data.secret_hash,
            &swap_data.swap_amount.to_le_bytes(),
            &swap_data.timelock.to_le_bytes(),
        ],
        bump = swap_data.bump,
        close = rent_sponsor,
    )]
    pub swap_data: Account<'info, SwapAccount>,

    /// A token account controlled by the program, escrowing the tokens for this atomic swap
    #[account(mut, token::mint = swap_data.mint, token::authority = identity_pda)]
    pub token_vault: Account<'info, TokenAccount>,

    /// CHECK: The token account of the redeemer
    #[account(mut, token::mint = swap_data.mint, token::authority = swap_data.redeemer)]
    pub redeemer_token_account: Account<'info, TokenAccount>,

    /// CHECK: Rent sponsor's address for refunding PDA rent
    #[account(mut, address = swap_data.rent_sponsor @ SwapError::InvalidRentSponsor)]
    pub rent_sponsor: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Refund<'info> {
    /// CHECK: The Identity PDA, used solely for authorizing token transfers, no data is read or written to it
    #[account(seeds = [], bump = swap_data.identity_pda_bump)]
    pub identity_pda: AccountInfo<'info>,

    /// The PDA holding the state information of the atomic swap. Will be closed upon successful execution
    /// and the resulting rent refund will be sent to the rent_sponsor.
    #[account(
        mut,
        seeds = [
            swap_data.mint.as_ref(),
            swap_data.redeemer.as_ref(),
            swap_data.refundee.as_ref(),
            &swap_data.secret_hash,
            &swap_data.swap_amount.to_le_bytes(),
            &swap_data.timelock.to_le_bytes(),
        ],
        bump = swap_data.bump,
        close = rent_sponsor,
    )]
    pub swap_data: Account<'info, SwapAccount>,

    /// A token account controlled by the program, escrowing the tokens for this atomic swap
    #[account(mut, token::mint = swap_data.mint, token::authority = identity_pda)]
    pub token_vault: Account<'info, TokenAccount>,

    /// CHECK: The token account of the refundee
    #[account(mut, token::mint = swap_data.mint, token::authority = swap_data.refundee)]
    pub refundee_token_account: Account<'info, TokenAccount>,

    /// CHECK: Rent sponsor's address for refunding PDA rent
    #[account(mut, address = swap_data.rent_sponsor @ SwapError::InvalidRentSponsor)]
    pub rent_sponsor: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct InstantRefund<'info> {
    /// CHECK: The Identity PDA, used solely for authorizing token transfers, no data is read or written to it
    #[account(seeds = [], bump = swap_data.identity_pda_bump)]
    pub identity_pda: AccountInfo<'info>,

    /// The PDA holding the state information of the atomic swap. Will be closed upon successful execution
    /// and the resulting rent refund will be sent to the rent_sponsor.
    #[account(
        mut,
        seeds = [
            swap_data.mint.as_ref(),
            swap_data.redeemer.as_ref(),
            swap_data.refundee.as_ref(),
            &swap_data.secret_hash,
            &swap_data.swap_amount.to_le_bytes(),
            &swap_data.timelock.to_le_bytes(),
        ],
        bump = swap_data.bump,
        close = rent_sponsor,
    )]
    pub swap_data: Account<'info, SwapAccount>,

    /// A token account controlled by the program, escrowing the tokens for this atomic swap
    #[account(mut, token::mint = swap_data.mint, token::authority = identity_pda)]
    pub token_vault: Account<'info, TokenAccount>,

    /// CHECK: The token account of the refundee
    #[account(mut, token::mint = swap_data.mint, token::authority = swap_data.refundee)]
    pub refundee_token_account: Account<'info, TokenAccount>,

    /// The redeemer of the atomic swap. They must sign this transaction.
    #[account(mut, address = swap_data.redeemer @ SwapError::InvalidRedeemer)]
    pub redeemer: Signer<'info>,

    /// CHECK: Rent sponsor's address for PDA rent refund
    #[account(mut, address = swap_data.rent_sponsor @ SwapError::InvalidRentSponsor)]
    pub rent_sponsor: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
}

/// Represents the initiated state of the swap where the funder has deposited funds into the vault
#[event]
pub struct Initiated {
    pub mint: Pubkey,
    pub redeemer: Pubkey,
    pub refundee: Pubkey,
    pub secret_hash: [u8; 32],
    /// The quantity of tokens transferred through this atomic swap in base units of the token mint.  
    /// E.g: A quantity of $1 represented by the token "USDC" with "6" decimals will be represented as 1,000,000.
    pub swap_amount: u64,
    /// `timelock` represents the number of slots after which (non-instant) refunds are allowed
    pub timelock: u64,
    /// Information regarding the destination chain in the atomic swap
    pub destination_data: Option<Vec<u8>>,
    pub funder: Pubkey,
}
/// Represents the redeemed state of the swap, where the redeemer has withdrawn funds from the vault
#[event]
pub struct Redeemed {
    pub mint: Pubkey,
    pub redeemer: Pubkey,
    pub refundee: Pubkey,
    pub secret: [u8; 32],
    pub swap_amount: u64,
    pub timelock: u64,
}
/// Represents the refund state of the swap, where the initiator has withdrawn funds from the vault past expiry
#[event]
pub struct Refunded {
    pub mint: Pubkey,
    pub redeemer: Pubkey,
    pub refundee: Pubkey,
    pub secret_hash: [u8; 32],
    pub swap_amount: u64,
    pub timelock: u64,
}
/// Represents the instant refund state of the swap, where the refundee has obtained
/// a refund of the funds with the redeemer's consent
#[event]
pub struct InstantRefunded {
    pub mint: Pubkey,
    pub redeemer: Pubkey,
    pub refundee: Pubkey,
    pub secret_hash: [u8; 32],
    pub swap_amount: u64,
    pub timelock: u64,
}

#[error_code]
pub enum SwapError {
    #[msg("The provider redeemer is not the original redeemer of this swap")]
    InvalidRedeemer,

    #[msg("The provided secret does not correspond to the secret hash of this swap")]
    InvalidSecret,

    #[msg("The provided rent_sponsor is not the original rent_sponsor of this swap")]
    InvalidRentSponsor,

    #[msg("Attempt to refund before timelock expiry")]
    RefundBeforeExpiry,
}
