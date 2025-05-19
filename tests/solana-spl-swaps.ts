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
    const swapExpiresIn = new anchor.BN(2); // 2 slots = 800 ms
    const secret: Buffer = crypto.randomBytes(32);
    const secretHash: Buffer = crypto.createHash('sha256').update(secret).digest();

    let mint: web3.PublicKey;  // A random mint for the test
    const mintAuthority = new web3.Keypair();  // And a random authority for it

    // Alice is the initiator here
    // Alice's stuff
    const alice = new web3.Keypair();
    let aliceTokenAccount: web3.PublicKey;

    // Bob's stuff
    const bob = new web3.Keypair();
    let bobTokenAccount: web3.PublicKey;

    const pdaSeeds = (phrase: string) => [Buffer.from(phrase), alice.publicKey.toBuffer(), secretHash];
    const [swapAccount,] = web3.PublicKey.findProgramAddressSync(pdaSeeds("swap_account"), program.programId);
    const [swapTokenAccount,] = web3.PublicKey.findProgramAddressSync(pdaSeeds("swap_token_account"), program.programId);

    let latestBlockHash: web3.BlockhashWithExpiryBlockHeight;

    before(async () => {
        latestBlockHash = await connection.getLatestBlockhash();
        // Fund alice's wallet with 1 SOL, her funds will be used for setting up the test as well
        const signature = await connection.requestAirdrop(alice.publicKey, 1_000_000_000);
        await connection.confirmTransaction({ signature, ...latestBlockHash });

        // Create Mint and Associated Token Accounts
        mint = await spl.createMint(connection, alice, mintAuthority.publicKey, null, 0);
        aliceTokenAccount = await spl.createAssociatedTokenAccount(connection, alice, mint, alice.publicKey);
        bobTokenAccount = await spl.createAssociatedTokenAccount(connection, alice, mint, bob.publicKey);

        // Fund alice's token wallet with 100 tokens
        await spl.mintTo(connection, alice, mint, aliceTokenAccount, mintAuthority, 100)
        await connection.confirmTransaction({ signature, ...latestBlockHash })

        console.log(
            `Account Information:
\tAlice:    ${alice.publicKey} \t Alice TokenWallet: \t${aliceTokenAccount}
\tBob:      ${bob.publicKey} \t Bob TokenWallet: \t${bobTokenAccount}
\tSwap Acc: ${swapAccount} \t Swap Wallet: \t\t${swapTokenAccount}
\tMint:     ${mint}\t Mint Authority: \t${mintAuthority.publicKey}\n`
        );
    });

    async function aliceInitiate() {
        const signature = await program.methods
            .initiate(swapAmount, swapExpiresIn, bobTokenAccount, [...secretHash])
            .accounts({
                initiator: alice.publicKey,
                initiatorTokenAccount: aliceTokenAccount,
                mint,
            })
            .signers([alice])
            .rpc();
        await connection.confirmTransaction({signature, ...latestBlockHash});
        console.log(`\tInitiate: \t${signature}`);
    }

    it("Test initiation", async () => {
        await aliceInitiate();
        const swapWalletBalance = (await connection.getTokenAccountBalance(swapTokenAccount)).value.amount;
        expect(swapWalletBalance).to.equal(swapAmount.toString());
    });

    it("Test redeem", async () => {
        // The previous testcase has initiated the swap
        const signature = await program.methods.redeem([...secret])
            .accounts({
                initiator: alice.publicKey,
                redeemerTokenAccount: bobTokenAccount,
                swapAccount,
                swapTokenAccount,
            })
            .rpc();
        await connection.confirmTransaction({signature, ...latestBlockHash});
        console.log(`\tRedeem: \t${signature}`);

        const bobBalance = (await connection.getTokenAccountBalance(bobTokenAccount)).value.amount;
        expect(bobBalance).to.equal(swapAmount.toString());
    });

    it("Test refund", async () => {
        await aliceInitiate();  // Re-initiating for this test
        const expiryMs = swapExpiresIn.toNumber() * 400;
        console.log(`Awaiting timelock of ${expiryMs}ms for Refund`);
        await new Promise(r => setTimeout(r, expiryMs + 400));
        const signature = await program.methods.refund()
            .accounts({
                swapAccount,
                swapTokenAccount: swapTokenAccount,
                initiator: alice.publicKey,
                initiatorTokenAccount: aliceTokenAccount,
            })
            .rpc();
        await connection.confirmTransaction({ signature, ...latestBlockHash });
        console.log(`\tRefund: \t${signature}`);

        // Alice had a token balance of 80 (-10 from above init(), -10 from previous swap)
        const aliceBalance = (await connection.getTokenAccountBalance(aliceTokenAccount)).value.amount;
        expect(aliceBalance).to.equal('90');  // Successful redeem adds +10 making it 90
    });

    it("Test instant refund", async () => {
        await aliceInitiate();  // Re-initiating for this test
        const signature = await program.methods.instantRefund()
            .accounts({
                initiator: alice.publicKey,
                initiatorTokenAccount: aliceTokenAccount,
                redeemer: bob.publicKey,
                redeemerTokenAccount: bobTokenAccount,
                swapAccount,
                swapTokenAccount: swapTokenAccount,
            })
            .signers([bob])
            .rpc();
        await connection.confirmTransaction({ signature, ...latestBlockHash });
        console.log(`\tInstant Refund:  ${signature}`);

        // Alice had a token balance of 80 (-10 from above init(), -10 from previous swap)
        const aliceBalance = (await connection.getTokenAccountBalance(aliceTokenAccount)).value.amount;
        expect(aliceBalance).to.equal('90');  // Successful redeem adds +10 making it 90
    });
});
