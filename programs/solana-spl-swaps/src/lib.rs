use anchor_lang::prelude::*;
use anchor_lang::solana_program::hash;
use anchor_spl::token::{self, Mint, Token, TokenAccount};

declare_id!("2WXpY8havGjfRxme9LUxtjFHTh1EfU3ur4v6wiK4KdNC");

#[program]
pub mod solana_spl_swaps {
    use super::*;

    pub fn initiate(
        ctx: Context<Initiate>,
        expires_in_slots: u64,
        redeemer: Pubkey,
        secret_hash: [u8; 32],
        swap_amount: u64, // In base units of the token
    ) -> Result<()> {
        let Initiate {
            initiator,
            initiator_token_account,
            sponsor,
            token_program,
            token_vault,
            ..
        } = ctx.accounts;

        *ctx.accounts.swap_data = SwapAccount {
            initiator: initiator.key(),
            expiry_slot: Clock::get()?.slot + expires_in_slots,
            redeemer,
            secret_hash,
            swap_amount,
            identity_pda_bump: ctx.bumps.identity_pda,
            sponsor: sponsor.key(),
        };

        let token_transfer_context = CpiContext::new(
            token_program.to_account_info(),
            token::Transfer {
                from: initiator_token_account.to_account_info(),
                to: token_vault.to_account_info(),
                authority: initiator.to_account_info(),
            },
        );
        token::transfer(token_transfer_context, swap_amount)?;

        emit!(Initiated {
            expires_in_slots,
            initiator: initiator.key(),
            redeemer,
            secret_hash,
            swap_amount,
        });

        Ok(())
    }

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
            initiator,
            redeemer,
            secret_hash,
            swap_amount,
            ..
        } = **swap_data;

        require!(
            redeemer_token_account.owner == redeemer,
            SwapError::InvalidRedeemer
        );
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

        emit!(Redeemed { initiator, secret });

        Ok(())
    }

    pub fn refund(ctx: Context<Refund>) -> Result<()> {
        let Refund {
            identity_pda,
            initiator_token_account,
            swap_data,
            token_vault,
            token_program,
            ..
        } = ctx.accounts;
        let SwapAccount {
            identity_pda_bump,
            initiator,
            expiry_slot,
            secret_hash,
            swap_amount,
            ..
        } = **swap_data;

        require!(
            initiator_token_account.owner == initiator,
            SwapError::InvalidInitiator
        );
        require!(
            Clock::get()?.slot > expiry_slot,
            SwapError::RefundBeforeExpiry
        );

        let pda_seeds: &[&[&[u8]]] = &[&[&[identity_pda_bump]]];
        let token_transfer_context = CpiContext::new(
            token_program.to_account_info(),
            token::Transfer {
                from: token_vault.to_account_info(),
                to: initiator_token_account.to_account_info(),
                authority: identity_pda.to_account_info(),
            },
        )
        .with_signer(pda_seeds);
        token::transfer(token_transfer_context, swap_amount)?;

        emit!(Refunded {
            initiator,
            secret_hash,
        });

        Ok(())
    }

    pub fn instant_refund(ctx: Context<InstantRefund>) -> Result<()> {
        let InstantRefund {
            identity_pda,
            initiator_token_account,
            swap_data,
            token_program,
            token_vault,
            ..
        } = ctx.accounts;
        let SwapAccount {
            identity_pda_bump,
            initiator,
            secret_hash,
            swap_amount,
            ..
        } = **swap_data;

        require!(
            initiator_token_account.owner == initiator,
            SwapError::InvalidInitiator
        );

        let pda_seeds: &[&[&[u8]]] = &[&[&[identity_pda_bump]]];
        let token_transfer_context = CpiContext::new(
            token_program.to_account_info(),
            token::Transfer {
                from: token_vault.to_account_info(),
                to: initiator_token_account.to_account_info(),
                authority: identity_pda.to_account_info(),
            },
        )
        .with_signer(pda_seeds);
        token::transfer(token_transfer_context, swap_amount)?;

        emit!(InstantRefunded {
            initiator,
            secret_hash
        });

        Ok(())
    }
}

#[account]
#[derive(InitSpace)]
pub struct SwapAccount {
    pub expiry_slot: u64,
    pub initiator: Pubkey,
    pub redeemer: Pubkey,
    pub secret_hash: [u8; 32],
    pub swap_amount: u64, // In base units of the token

    pub identity_pda_bump: u8, // Needed for authorizing token transfers
    pub sponsor: Pubkey,
}

#[derive(Accounts)]
// Make sure the parameters are in exact name and order as that of the function, otherwise you will get
// a seed constraint violated error.
// Refer: https://www.anchor-lang.com/docs/references/account-constraints#instruction-attribute
#[instruction(expires_in_slots: u64, redeemer_token_account: Pubkey, secret_hash: [u8; 32])]
pub struct Initiate<'info> {
    /// CHECK: A permanent PDA that represents this swap program for authorizing
    /// the token transfers of the `token_vault` PDA.
    #[account(
        init_if_needed,
        payer = sponsor,
        seeds = [],
        bump,
        space = 0,
    )]
    pub identity_pda: AccountInfo<'info>,

    /// A PDA that maintains the on-chain state of the atomic swap throughout its lifecycle.  
    /// The choice of seeds ensures that any swap with equal `initiator` and
    /// `secret_hash` cannot be created until an existing one finishes.  
    /// This PDA will be deleted upon completion of the swap.
    #[account(
        init,
        payer = sponsor,
        seeds = [initiator.key().as_ref(), &secret_hash],
        bump,
        space = 8 + SwapAccount::INIT_SPACE,
    )]
    pub swap_data: Account<'info, SwapAccount>,

    /// A permanent PDA that serves as the "vault" by escrowing tokens of type `mint`
    /// for the atomic swap. It is intended to be reused for all swaps involving the same mint.
    #[account(
        init_if_needed,
        payer = sponsor,
        seeds = [mint.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = identity_pda,
    )]
    pub token_vault: Account<'info, TokenAccount>,

    // The initiator must sign this transaction
    pub initiator: Signer<'info>,

    #[account(
        mut,
        token::mint = mint,
        token::authority = initiator,
    )]
    pub initiator_token_account: Account<'info, TokenAccount>,

    pub mint: Account<'info, Mint>,

    /// Sponsors the transaction fees and PDA rent.
    /// Upon completion of the swap, the PDA rent will be refunded to this address.
    #[account(mut)]
    pub sponsor: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Redeem<'info> {
    /// CHECK: The Identity PDA, used only for authorizing token transfers, no data is read or written.    
    #[account(seeds = [], bump = swap_data.identity_pda_bump)]
    pub identity_pda: AccountInfo<'info>,

    #[account(mut, close = sponsor)]
    pub swap_data: Account<'info, SwapAccount>,

    #[account(mut, token::authority = identity_pda)]
    pub token_vault: Account<'info, TokenAccount>,

    /// CHECK: Verification is done in the function
    #[account(mut)]
    pub redeemer_token_account: Account<'info, TokenAccount>,

    /// CHECK: Sponsor's address for refunding PDA rent
    #[account(mut, address = swap_data.sponsor @ SwapError::InvalidSponsor)]
    pub sponsor: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Refund<'info> {
    /// CHECK: The Identity PDA, used only for authorizing token transfers, no data is read or written.    
    #[account(seeds = [], bump = swap_data.identity_pda_bump)]
    pub identity_pda: AccountInfo<'info>,

    #[account(mut, close = sponsor)]
    pub swap_data: Account<'info, SwapAccount>,

    #[account(mut, token::authority = identity_pda)]
    pub token_vault: Account<'info, TokenAccount>,

    /// CHECK: Verification is done in the function
    #[account(mut)]
    pub initiator_token_account: Account<'info, TokenAccount>,

    /// CHECK: Sponsor's address for refunding PDA rent
    #[account(mut, address = swap_data.sponsor @ SwapError::InvalidSponsor)]
    pub sponsor: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct InstantRefund<'info> {
    /// CHECK: The Identity PDA, used only for authorizing token transfers, no data is read or written.
    #[account(seeds = [], bump = swap_data.identity_pda_bump)]
    pub identity_pda: AccountInfo<'info>,

    #[account(mut, close = sponsor)]
    pub swap_data: Account<'info, SwapAccount>,

    #[account(mut, token::authority = identity_pda)]
    pub token_vault: Account<'info, TokenAccount>,

    /// CHECK: The authority is checked within the function
    #[account(mut)]
    pub initiator_token_account: Account<'info, TokenAccount>,

    /// Redeemer must sign this transaction
    #[account(mut, address = swap_data.redeemer @ SwapError::InvalidRedeemer)]
    pub redeemer: Signer<'info>,

    /// CHECK: Sponsor's address for PDA rent refund
    #[account(mut, address = swap_data.sponsor @ SwapError::InvalidSponsor)]
    pub sponsor: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
}

#[event]
pub struct Initiated {
    pub swap_amount: u64,
    pub expires_in_slots: u64,
    pub initiator: Pubkey,
    pub redeemer: Pubkey,
    pub secret_hash: [u8; 32],
}
#[event]
pub struct Redeemed {
    pub initiator: Pubkey,
    pub secret: [u8; 32],
}
#[event]
pub struct Refunded {
    pub initiator: Pubkey,
    pub secret_hash: [u8; 32],
}
#[event]
pub struct InstantRefunded {
    pub initiator: Pubkey,
    pub secret_hash: [u8; 32],
}

#[error_code]
pub enum SwapError {
    #[msg("The provided token account does not belong to the initiator of this swap account")]
    InvalidInitiator,

    #[msg("The provided token account does not belong to the redeemer of this swap account")]
    InvalidRedeemer,

    #[msg("The provided secret does not correspond to the secret hash in the swap account")]
    InvalidSecret,

    #[msg("The provided sponsor is not the original sponsor of this swap")]
    InvalidSponsor,

    #[msg("Attempt to perform a refund before expiry time")]
    RefundBeforeExpiry,
}
