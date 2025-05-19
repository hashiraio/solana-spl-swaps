use anchor_lang::prelude::*;
use anchor_lang::solana_program::hash;
use anchor_spl::{
    token,
    token::{Mint, Token, TokenAccount},
};
declare_id!("2WXpY8havGjfRxme9LUxtjFHTh1EfU3ur4v6wiK4KdNC");

#[program]
pub mod solana_spl_swaps {
    use super::*;

    pub fn initiate(
        ctx: Context<Initiate>,
        swap_amount: u64, // In base units of the token
        expires_in_slots: u64,
        redeemer_token_account: Pubkey,
        secret_hash: [u8; 32],
    ) -> Result<()> {
        let Initiate {
            initiator,
            initiator_token_account,
            swap_token_account,
            token_program,
            ..
        } = ctx.accounts;

        *ctx.accounts.swap_account = SwapAccount {
            expiry_slot: Clock::get()?.slot + expires_in_slots,
            initiator: initiator.key(),
            redeemer_token_account,
            secret_hash,
            swap_amount,
            bump: ctx.bumps.swap_account,
        };

        let cpi_context = CpiContext::new(
            token_program.to_account_info(),
            token::Transfer {
                from: initiator_token_account.to_account_info(),
                to: swap_token_account.to_account_info(),
                authority: initiator.to_account_info(),
            },
        );
        token::transfer(cpi_context, swap_amount)?;

        emit!(Initiated {
            initiator: initiator.key(),
            expires_in_slots,
            redeemer_token_account,
            secret_hash,
            swap_amount,
        });

        Ok(())
    }

    pub fn redeem(ctx: Context<Redeem>, secret: [u8; 32]) -> Result<()> {
        let Redeem {
            swap_token_account,
            swap_account,
            initiator,
            redeemer_token_account,
            token_program,
            ..
        } = ctx.accounts;
        let SwapAccount {
            bump,
            secret_hash,
            swap_amount,
            ..
        } = **swap_account;

        require!(
            hash::hash(&secret).to_bytes() == secret_hash,
            SwapError::InvalidSecret
        );

        let initiator_key = initiator.key();
        let pda_seeds: &[&[&[u8]]] = &[&[
            b"swap_account",
            initiator_key.as_ref(),
            &secret_hash,
            &[bump],
        ]];
        // Transfer the tokens to the redeemer
        let cpi_context = CpiContext::new(
            token_program.to_account_info(),
            token::Transfer {
                from: swap_token_account.to_account_info(),
                to: redeemer_token_account.to_account_info(),
                authority: swap_account.to_account_info(),
            },
        )
        .with_signer(pda_seeds);
        token::transfer(cpi_context, swap_amount)?;

        emit!(Redeemed {
            initiator: initiator.key(),
            secret
        });

        // Close the swap wallet (returns rent lamports to initiator)
        let cpi_context = CpiContext::new(
            token_program.to_account_info(),
            token::CloseAccount {
                account: swap_token_account.to_account_info(),
                destination: initiator.to_account_info(),
                authority: swap_account.to_account_info(),
            },
        )
        .with_signer(pda_seeds);
        token::close_account(cpi_context)?;

        Ok(())
    }

    pub fn refund(ctx: Context<Refund>) -> Result<()> {
        let Refund {
            token_program,
            swap_account,
            swap_token_account,
            initiator,
            initiator_token_account,
            ..
        } = ctx.accounts;
        let SwapAccount {
            bump,
            expiry_slot,
            secret_hash,
            swap_amount,
            ..
        } = **swap_account;

        require!(
            Clock::get()?.slot > expiry_slot,
            SwapError::RefundBeforeExpiry
        );

        let initiator_key = initiator.key();
        let pda_seeds: &[&[&[u8]]] = &[&[
            b"swap_account",
            initiator_key.as_ref(),
            &secret_hash,
            &[bump],
        ]];
        let cpi_context = CpiContext::new(
            token_program.to_account_info(),
            token::Transfer {
                from: swap_token_account.to_account_info(),
                to: initiator_token_account.to_account_info(),
                authority: swap_account.to_account_info(),
            },
        )
        .with_signer(pda_seeds);
        token::transfer(cpi_context, swap_amount)?;

        emit!(Refunded {
            initiator: initiator_key,
            secret_hash,
        });

        let cpi_context = CpiContext::new(
            token_program.to_account_info(),
            token::CloseAccount {
                account: swap_token_account.to_account_info(),
                destination: initiator.to_account_info(),
                authority: swap_account.to_account_info(),
            },
        )
        .with_signer(pda_seeds);
        token::close_account(cpi_context)?;

        Ok(())
    }

    pub fn instant_refund(ctx: Context<InstantRefund>) -> Result<()> {
        let InstantRefund {
            token_program,
            swap_account,
            swap_token_account,
            initiator,
            initiator_token_account,
            ..
        } = ctx.accounts;
        let SwapAccount {
            bump,
            secret_hash,
            swap_amount,
            ..
        } = **swap_account;

        let initiator_key = initiator.key();
        let pda_seeds: &[&[&[u8]]] = &[&[
            b"swap_account",
            initiator_key.as_ref(),
            &secret_hash,
            &[bump],
        ]];

        let cpi_context = CpiContext::new(
            token_program.to_account_info(),
            token::Transfer {
                from: swap_token_account.to_account_info(),
                to: initiator_token_account.to_account_info(),
                authority: swap_account.to_account_info(),
            },
        )
        .with_signer(pda_seeds);
        token::transfer(cpi_context, swap_amount)?;

        emit!(InstantRefunded {
            initiator: initiator_key,
            secret_hash
        });

        // Close the wallet, refunding rent lamports to initiator
        let cpi_context = CpiContext::new(
            token_program.to_account_info(),
            token::CloseAccount {
                account: swap_token_account.to_account_info(),
                destination: initiator.to_account_info(),
                authority: swap_account.to_account_info(),
            },
        )
        .with_signer(pda_seeds);
        token::close_account(cpi_context)?;

        Ok(())
    }
}

#[account]
#[derive(InitSpace)]
pub struct SwapAccount {
    pub bump: u8, // Needed for PDA signatures
    pub expiry_slot: u64,
    pub initiator: Pubkey,
    pub redeemer_token_account: Pubkey,
    pub secret_hash: [u8; 32],
    pub swap_amount: u64, // In base units of the token
}

#[derive(Accounts)]
// Refer: https://www.anchor-lang.com/docs/references/account-constraints#instruction-attribute
#[instruction(swap_amount: u64, expires_in_slots: u64, redeemer_token_account: Pubkey, secret_hash: [u8; 32])]
pub struct Initiate<'info> {
    #[account(
        init,
        payer = initiator,
        seeds = [b"swap_account", initiator.key().as_ref(), &secret_hash],
        bump,
        space = 8 + SwapAccount::INIT_SPACE,
    )]
    pub swap_account: Account<'info, SwapAccount>,

    #[account(
        init,
        payer = initiator,
        seeds = [b"swap_token_account", initiator.key().as_ref(), &secret_hash],
        bump,
        token::mint = mint,
        token::authority = swap_account,
    )]
    pub swap_token_account: Account<'info, TokenAccount>,

    // The initiator must sign this transaction
    #[account(mut)]
    pub initiator: Signer<'info>,

    #[account(
        mut,
        token::mint = mint,
        token::authority = initiator,
    )]
    pub initiator_token_account: Account<'info, TokenAccount>,

    pub mint: Account<'info, Mint>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Redeem<'info> {
    #[account(mut, close = initiator)]
    pub swap_account: Account<'info, SwapAccount>,

    #[account(mut, token::authority = swap_account)]
    pub swap_token_account: Account<'info, TokenAccount>,

    #[account(mut, address = swap_account.redeemer_token_account @ SwapError::InvalidRedeemer)]
    pub redeemer_token_account: Account<'info, TokenAccount>,

    /// CHECK: Initiator's address for refunding PDA rent amounts
    #[account(mut, address = swap_account.initiator @ SwapError::InvalidInitiator)]
    pub initiator: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Refund<'info> {
    #[account(mut, close = initiator)]
    pub swap_account: Account<'info, SwapAccount>,

    #[account(mut, token::authority = swap_account)]
    pub swap_token_account: Account<'info, TokenAccount>,

    /// CHECK: Initiator's address for refunding PDA rent amounts
    #[account(mut, address = swap_account.initiator @ SwapError::InvalidInitiator)]
    pub initiator: AccountInfo<'info>,

    #[account(mut)]
    pub initiator_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct InstantRefund<'info> {
    #[account(mut, close = initiator)]
    pub swap_account: Account<'info, SwapAccount>,

    #[account(mut, token::authority = swap_account)]
    pub swap_token_account: Account<'info, TokenAccount>,

    /// CHECK: Initiator's address for PDA rent refund
    #[account(mut)]
    pub initiator: AccountInfo<'info>,

    #[account(mut, token::authority = initiator)]
    pub initiator_token_account: Account<'info, TokenAccount>,

    /// CHECK: Redeemer's address for signature verification
    #[account(mut)]
    pub redeemer: Signer<'info>,

    // `redeemer` must be the authority for the swap's `redeemer_token_account`
    #[account(
        mut,
        token::authority = redeemer,
        address = swap_account.redeemer_token_account @ SwapError::InvalidRedeemer,
    )]
    pub redeemer_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[event]
pub struct Initiated {
    pub swap_amount: u64,
    pub expires_in_slots: u64,
    pub initiator: Pubkey,
    pub redeemer_token_account: Pubkey,
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
    #[msg("The provided initiator is not the original initiator of this swap account")]
    InvalidInitiator,

    #[msg("The provided redeemer is not the original redeemer of this swap amount")]
    InvalidRedeemer,

    #[msg("The provided secret does not correspond to the secret hash in the swap account")]
    InvalidSecret,

    #[msg("Attempt to perform a refund before expiry time")]
    RefundBeforeExpiry,
}
