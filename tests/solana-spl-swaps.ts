import * as anchor from "@coral-xyz/anchor";
import { Program, web3 } from "@coral-xyz/anchor";
import * as spl from "@solana/spl-token";

import { expect } from "chai";
import * as crypto from 'crypto';

import { SolanaSplSwaps } from "../target/types/solana_spl_swaps";

// Configure the client to use the local cluster.
anchor.setProvider(anchor.AnchorProvider.env());
const connection = anchor.getProvider().connection;
const program = anchor.workspace.SolanaSplSwaps as Program<SolanaSplSwaps>;

describe("Testing one way swap between Alice and Bob", () => {
    const swapAmount = new anchor.BN(10);
    const expiryMs = 1000;
    // Expiry must be provided in Slot units where 1 slot = 400 ms
    const swapExpiresIn = expiryMs / 400;
    const secret: Buffer = crypto.randomBytes(32);
    const secretHash: Buffer = crypto.createHash('sha256').update(secret).digest();

    let mint: web3.PublicKey;  // A random mint for the test
    const mintAuthority = new web3.Keypair();  // And a random authority for it
    const mintAuthorityPubkey = mintAuthority.publicKey;  // And his pubkey

    // Alice is the initiator here
    // Alice's stuff
    const alice = new web3.Keypair();     // Alice's Solana Keypair
    const alicePubkey = alice.publicKey;  // Alice's Solan address
    let aliceWallet: web3.PublicKey;      // Alice's Token Wallet

    const swapIdPreImage = Buffer.concat([alicePubkey.toBuffer(), secretHash]);
    const swapId = crypto.createHash('sha256').update(swapIdPreImage).digest();
    // Bob's stuff
    const bob = new web3.Keypair();       // Bob's Solana Keypair
    const bobPubkey = bob.publicKey;      // Bob's Solana Address
    let bobWallet: web3.PublicKey;        // Bob's Token Wallet

    // Deriving PDAs, just for logging and test verification
    const pdaSeeds = (phrase: string) => [Buffer.from(phrase), swapId];
    const [swapAccount,] = web3.PublicKey.findProgramAddressSync(pdaSeeds("swap_account"), program.programId);
    const [swapWallet,] = web3.PublicKey.findProgramAddressSync(pdaSeeds("swap_wallet"), program.programId);

    before(async () => {
        // Fund alice's wallet with 1 SOL, her funds will be used for setting up the test as well
        await connection.requestAirdrop(alicePubkey, 1_000_000_000)
            .then(async signature =>
                await connection.confirmTransaction({ signature, ...(await connection.getLatestBlockhash()) })
            );

        // Create Mint and Associated Token Accounts
        mint = await spl.createMint(connection, alice, mintAuthorityPubkey, null, 0);
        aliceWallet = await spl.createAssociatedTokenAccount(connection, alice, mint, alicePubkey);
        bobWallet = await spl.createAssociatedTokenAccount(connection, alice, mint, bobPubkey);

        // Fund alice's token wallet with 100 tokens
        await spl.mintTo(connection, alice, mint, aliceWallet, mintAuthority, 100)
            .then(async signature =>
                await connection.confirmTransaction({ signature, ...(await connection.getLatestBlockhash()) })
            );
        console.log(
            `Account Information:
\tAlice:    ${alicePubkey} \t Alice TokenWallet: \t${aliceWallet}
\tBob:      ${bobPubkey} \t Bob TokenWallet: \t${bobWallet}
\tSwap Acc: ${swapAccount} \t Swap Wallet: \t\t${swapWallet}
\tMint:     ${mint}\t Mint Authority: \t${mintAuthorityPubkey}\n`
        );
    });

    async function aliceInitiate() {
        await program.methods
            .initiate([...swapId], [...secretHash], bobWallet, swapAmount, swapExpiresIn)
            .accounts({
                initiator: alicePubkey,
                initiatorWallet: aliceWallet,
                mint,
            })
            .signers([alice]).rpc()
            .then(async signature => {
                console.log(`\tInitiate: \t${signature}`);
                await connection.confirmTransaction({signature, ...(await connection.getLatestBlockhash())});
            });
    }

    it("Test initiation", async () => {
        await aliceInitiate();
        const swapWalletBalance = (await connection.getTokenAccountBalance(swapWallet)).value.amount;
        expect(swapWalletBalance).to.equal(swapAmount.toString());
    });

    it("Test redeem", async () => {
        // The previous testcase has initiated the swap
        await program.methods.redeem([...secret])
        .accounts({
            swapAccount,
            swapWallet,
            redeemerWallet: bobWallet,
            initiator: alicePubkey,
        }).rpc().then(async signature => {
            console.log(`\tRedeem: \t${signature}`);
            await connection.confirmTransaction({signature, ...(await connection.getLatestBlockhash())});
        });
        const bobBalance = (await connection.getTokenAccountBalance(bobWallet)).value.amount;
        expect(bobBalance).to.equal(swapAmount.toString());
    });

    it("Test refund", async () => {
        await aliceInitiate();  // Re-initiating for this test
        console.log(`Awaiting timelock of ${expiryMs} for Refund`);
        await new Promise(r => setTimeout(r, expiryMs + 400));
        await program.methods.refund()
        .accounts({
            swapAccount,
            swapWallet,
            initiator: alicePubkey,
            initiatorWallet: aliceWallet,
        }).rpc().then(async signature => {
            console.log(`\tRefund: \t${signature}`);
            await connection.confirmTransaction({ signature, ...(await connection.getLatestBlockhash()) });
        });
        // Alice had a token balance of 80 (-10 from above init(), -10 from previous swap)
        const aliceBalance = (await connection.getTokenAccountBalance(aliceWallet)).value.amount;
        expect(aliceBalance).to.equal('90');  // Successful redeem adds +10 making it 90
    });

    it("Test instant refund", async () => {
        await aliceInitiate();  // Re-initiating for this test
        await program.methods.instantRefund()
        .accounts({
            swapAccount,
            swapWallet,
            initiator: alicePubkey,
            initiatorWallet: aliceWallet,
            redeemer: bobPubkey,
            redeemerWallet: bobWallet,
        }).signers([alice, bob])
        .rpc().then(async signature => {
            console.log(`\tInstant Refund:  ${signature}`);
            await connection.confirmTransaction({ signature, ...(await connection.getLatestBlockhash()) });
        });
        // Alice had a token balance of 80 (-10 from above init(), -10 from previous swap)
        const aliceBalance = (await connection.getTokenAccountBalance(aliceWallet)).value.amount;
        expect(aliceBalance).to.equal('90');  // Successful redeem adds +10 making it 90
    });
});
