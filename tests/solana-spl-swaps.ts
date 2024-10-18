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
const MILLIS_PER_SLOT = 400;

describe("Testing one way swap between Alice and Bob", () => {
    const swapAmount = new anchor.BN(10);
    const swapExpiresIn = 1000 / MILLIS_PER_SLOT; // 1 second
    const secret: Uint8Array = crypto.randomBytes(32);
    const secretHash = [...crypto.createHash('sha256').update(secret).digest()];

    let mint: web3.PublicKey;  // A random mint for the test
    const mintAuthority = new web3.Keypair();  // And a random authority for it
    const mintAuthorityPubkey = mintAuthority.publicKey;  // And his pubkey

    // Alice is the initiator here
    // Alice's stuff
    const alice = new web3.Keypair();     // Alice's Solana Keypair
    const alicePubkey = alice.publicKey;  // Alice's Solan address
    let aliceWallet: web3.PublicKey;      // Alice's Token Wallet

    const swapIdPreImage = Buffer.from([...alicePubkey.toBytes(), ...secretHash]);
    const swapId = [...crypto.createHash('sha256').update(swapIdPreImage).digest()];
    // Bob's stuff
    const bob = new web3.Keypair();       // Bob's Solana Keypair
    const bobPubkey = bob.publicKey;      // Bob's Solana Address
    let bobWallet: web3.PublicKey;        // Bob's Token Wallet

    // Deriving PDAs, just for logging and test verification
    const pdaSeeds = (phrase: string) => [Buffer.from(phrase), alicePubkey.toBuffer(), Buffer.from(secretHash)];
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
        await program.methods.initiate(secretHash, swapId, bobWallet, swapAmount, swapExpiresIn)
            .accounts({
                initiator: alicePubkey,
                initiatorWallet: aliceWallet,
                mint,
            })
            .signers([alice]).rpc()
            .then(async signature => {
                console.log(`\tInitiate: ${signature}`);
                await connection.confirmTransaction({signature, ...(await connection.getLatestBlockhash())});
            });
    }

    it("Test initiation", async () => {
        await aliceInitiate();
        const swapWalletBalance = (await connection.getTokenAccountBalance(swapWallet)).value.amount;
        expect(swapWalletBalance).to.equal(swapAmount.toString());
    });

    it("Test redeem", async () => {
        console.log("\tRedeem:  ", await program.methods.redeem([...secret])
        .accounts({
            swapAccount,
            swapWallet,
            redeemerWallet: bobWallet,
            initiator: alicePubkey,
        })
        .rpc());
    })
});
