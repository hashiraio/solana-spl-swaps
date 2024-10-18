use anchor_lang::prelude::*;
use anchor_lang::solana_program::hash;
use anchor_spl::{
    token,
    token::{Mint, Token, TokenAccount},
};
declare_id!("FeEiy23gsDpr6W7sxzcxEDJVXayYRNhkfBrkwLg2oYgn");

type Lamports = u64;
type Slots = u32;

#[program]
pub mod solana_spl_swaps {
    use super::*;

    pub fn initiate(
        ctx: Context<Initiate>,
        swap_id: [u8; 32],
        secret_hash: [u8; 32],
        redeemer_wallet: Pubkey,
        amount: Lamports,
        expires_in: Slots,
    ) -> Result<()> {
        let Initiate {
            initiator,
            initiator_wallet,
            swap_wallet,
            token_program,
            ..
        } = ctx.accounts;
        *ctx.accounts.swap_account = SwapAccount {
            swap_id,
            initiator: initiator.key(),
            redeemer_wallet,
            secret_hash,
            expiry_slot: Clock::get()?.slot + expires_in as u64,
            amount,
            bump: ctx.bumps.swap_account,
        };
        let cpi_context = CpiContext::new(
            token_program.to_account_info(),
            token::Transfer {
                from: initiator_wallet.to_account_info(),
                to: swap_wallet.to_account_info(),
                authority: initiator.to_account_info(),
            }
        );
        token::transfer(cpi_context, amount)?;
        emit!(Initiated { swap_id, secret_hash, amount });

        Ok(())
    }

    pub fn redeem(ctx: Context<Redeem>, secret: [u8; 32]) -> Result<()> {
        let Redeem {
            swap_wallet,
            swap_account,
            initiator,
            redeemer_wallet,
            token_program,
            ..
         } = ctx.accounts;
        let SwapAccount {
            swap_id,
            secret_hash,
            bump,
            amount,
            ..
        } = **swap_account;

        require!(hash::hash(&secret).as_ref() == &secret_hash, SwapError::InvalidSecret);

        let pda_seeds: &[&[&[u8]]] = &[&[ b"swap_account", &swap_id, &[bump] ]];
        // Transfer the tokens to the redeemer
        let cpi_context = CpiContext::new(
            token_program.to_account_info(),
            token::Transfer {
                from: swap_wallet.to_account_info(),
                to: redeemer_wallet.to_account_info(),
                authority: swap_account.to_account_info(),
            }
        ).with_signer(pda_seeds);
        token::transfer(cpi_context, amount)?;

        emit!(Redeemed { swap_id, secret });
        // Close the swap wallet (returns rent lamports to initiator)
        let cpi_context = CpiContext::new(
            token_program.to_account_info(),
            token::CloseAccount {
                account: swap_wallet.to_account_info(),
                destination: initiator.to_account_info(),
                authority: swap_account.to_account_info(),
            }
        ).with_signer(pda_seeds);
        token::close_account(cpi_context)?;

        Ok(())
    }

    pub fn refund(ctx: Context<Refund>) -> Result<()> {
        let Refund {
            token_program,
            swap_account,
            swap_wallet,
            initiator,
            initiator_wallet,
            ..
        } = ctx.accounts;
        let SwapAccount { expiry_slot, swap_id, amount, bump, .. } = **swap_account;

        require!(Clock::get()?.slot >= expiry_slot, SwapError::RefundBeforeExpiry);

        let pda_seeds: &[&[&[u8]]] = &[&[ b"swap_account", &swap_id, &[bump] ]];
        let cpi_context = CpiContext::new(
            token_program.to_account_info(),
            token::Transfer {
                from: swap_wallet.to_account_info(),
                to: initiator_wallet.to_account_info(),
                authority: swap_account.to_account_info(),
            }
        ).with_signer(pda_seeds);
        token::transfer(cpi_context, amount)?;

        emit!(Refunded { swap_id });

        let cpi_context = CpiContext::new(
            token_program.to_account_info(),
            token::CloseAccount {
                account: swap_wallet.to_account_info(),
                destination: initiator.to_account_info(),
                authority: swap_account.to_account_info(),
            }
        ).with_signer(pda_seeds);
        token::close_account(cpi_context)?;

        Ok(())
    }

    pub fn instant_refund(ctx: Context<InstantRefund>) -> Result<()> {
        let InstantRefund {
            token_program,
            swap_account,
            swap_wallet,
            initiator,
            initiator_wallet,
            ..
        } = ctx.accounts;
        let SwapAccount { swap_id, amount, bump, .. } = **swap_account;

        let pda_seeds: &[&[&[u8]]] = &[&[ b"swap_account", &swap_id, &[bump]]];

        let cpi_context = CpiContext::new(
            token_program.to_account_info(),
            token::Transfer {
                from: swap_wallet.to_account_info(),
                to: initiator_wallet.to_account_info(),
                authority: swap_account.to_account_info(),
            }).with_signer(pda_seeds);
        token::transfer(cpi_context, amount)?;

        emit!(InstantRefunded { swap_id });

        // Close the wallet, refunding rent lamports to initiator
        let cpi_context = CpiContext::new(
            token_program.to_account_info(),
            token::CloseAccount {
                account: swap_wallet.to_account_info(),
                destination: initiator.to_account_info(),
                authority: swap_account.to_account_info(),
            }).with_signer(pda_seeds);
        token::close_account(cpi_context)?;

        Ok(())
    }
}

#[account]
pub struct SwapAccount {
    swap_id: [u8; 32],
    initiator: Pubkey,
    redeemer_wallet: Pubkey, // Redeemer's token wallet
    secret_hash: [u8; 32],
    expiry_slot: u64,
    amount: Lamports,
    bump: u8,  // Used for pda_seeds in refund & redeem
}

#[derive(Accounts)]
#[instruction(swap_id: [u8; 32])]
pub struct Initiate<'info> {
    #[account(
        init,
        payer = initiator,
        seeds = [b"swap_account".as_ref(), &swap_id],
        bump,
        space = 8 + std::mem::size_of::<SwapAccount>(),
    )]
    pub swap_account: Account<'info, SwapAccount>,

    #[account(
        init,
        payer = initiator,
        seeds = [b"swap_wallet".as_ref(), &swap_id],
        bump,
        token::mint = mint,
        token::authority = swap_account,
    )]
    pub swap_wallet: Account<'info, TokenAccount>,

    #[account(mut)]
    pub initiator: Signer<'info>,

    #[account(
        mut,
        token::mint = mint,
        token::authority = initiator,
    )]
    pub initiator_wallet: Account<'info, TokenAccount>,

    pub mint: Account<'info, Mint>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Redeem<'info> {
    #[account(mut, close = initiator)]
    pub swap_account: Account<'info, SwapAccount>,

    #[account(mut, token::authority = swap_account)]
    pub swap_wallet: Account<'info, TokenAccount>,

    #[account(mut, address = swap_account.redeemer_wallet @ SwapError::InvalidRedeemer)]
    pub redeemer_wallet: Account<'info, TokenAccount>,

    /// CHECK: Initiator's address for refunding PDA rent amounts
    #[account(mut, address = swap_account.initiator @ SwapError::InvalidRefundee)]
    pub initiator: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Refund<'info> {
    #[account(mut, close = initiator)]
    pub swap_account: Account<'info, SwapAccount>,

    #[account(mut, token::authority = swap_account)]
    pub swap_wallet: Account<'info, TokenAccount>,

    /// CHECK: Initiator's address for refunding PDA rent amounts
    #[account(mut, address = swap_account.initiator @ SwapError::InvalidRefundee)]
    pub initiator: AccountInfo<'info>,

    #[account(mut)]
    pub initiator_wallet: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct InstantRefund<'info> {
    #[account(mut, close = initiator)]
    pub swap_account: Account<'info, SwapAccount>,

    #[account(mut, token::authority = swap_account)]
    pub swap_wallet: Account<'info, TokenAccount>,

    /// CHECK: Initiator's address for PDA rent refund and signature verification
    #[account(mut)]
    pub initiator: Signer<'info>,

    #[account(mut, token::authority = initiator)]
    pub initiator_wallet: Account<'info, TokenAccount>,

    /// CHECK: Redeemer's address for signature verification
    #[account(mut)]
    pub redeemer: Signer<'info>,

    #[account(
        mut,
        token::authority = redeemer,
        address = swap_account.redeemer_wallet @ SwapError::InvalidRedeemer,
    )]
    pub redeemer_wallet: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[event]
pub struct Initiated {
    swap_id: [u8; 32],
    secret_hash: [u8; 32],
    amount: u64,
}
#[event]
pub struct Redeemed {
    swap_id: [u8; 32],
    secret: [u8; 32],
}
#[event]
pub struct Refunded {
    swap_id: [u8; 32],
}
#[event]
pub struct InstantRefunded {
    swap_id: [u8; 32],
}

#[error_code]
pub enum SwapError {
    #[msg("The provided redeemer is not the intended recipient of the swap amount")]
    InvalidRedeemer,

    #[msg("The provided initiator/refundee is not the original initiator of the given swap account")]
    InvalidRefundee,

    #[msg("The provided secret does not correspond to the secret hash in the swap account")]
    InvalidSecret,

    #[msg("Attempt to perform a refund before expiry time")]
    RefundBeforeExpiry,
}
