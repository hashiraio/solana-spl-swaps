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
        let swap_id = hash::hash(
            &[initiator.key().as_ref(), &secret_hash].concat()
        ).to_bytes();
        *ctx.accounts.swap_account = SwapAccount {
            swap_id,
            initiator: initiator.key(),
            redeemer_wallet,
            secret_hash,
            expiry_slot: Clock::get()?.slot + expires_in as u64,
            amount,
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
}

#[account]
pub struct SwapAccount {
    swap_id: [u8; 32],
    initiator: Pubkey,
    redeemer_wallet: Pubkey, // Redeemer's token wallet
    secret_hash: [u8; 32],
    expiry_slot: u64,
    amount: Lamports,
}

#[derive(Accounts)]
#[instruction(secret_hash: [u8; 32])]
pub struct Initiate<'info> {
    #[account(
        init,
        payer = initiator,
        seeds = [b"swap_account".as_ref(), initiator.key().as_ref(), secret_hash.as_ref()],
        bump,
        space = 8 + std::mem::size_of::<SwapAccount>(),
    )]
    pub swap_account: Account<'info, SwapAccount>,

    #[account(
        init,
        payer = initiator,
        seeds = [b"swap_wallet".as_ref(), initiator.key().as_ref(), secret_hash.as_ref()],
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

#[event]
pub struct Initiated {
    swap_id: [u8; 32],
    secret_hash: [u8; 32],
    amount: u64,
}
