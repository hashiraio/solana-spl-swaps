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

    const mint = web3.Keypair.fromSeed(new Uint8Array(32).fill(33));
    const mintAuthority = web3.Keypair.fromSeed(new Uint8Array(32).fill(36));

    // Alice, the initiator
    const alice = new web3.Keypair();
    let aliceTokenAccount: web3.PublicKey;

    // Bob, the redeemer
    const bob = new web3.Keypair();
    let bobTokenAccount: web3.PublicKey;

    const [swapAccount,] = web3.PublicKey.findProgramAddressSync(
        [Buffer.from("swap_account"), alice.publicKey.toBuffer(), secretHash],
        program.programId
    );
    const [swapTokenAccount,] = web3.PublicKey.findProgramAddressSync([mint.publicKey.toBuffer()], program.programId);

    let latestBlockHash: web3.BlockhashWithExpiryBlockHeight;

    before(async () => {
        latestBlockHash = await connection.getLatestBlockhash();
        // Fund alice's token acc with 1 SOL, these will also be used for setting up the test
        const signature = await connection.requestAirdrop(alice.publicKey, web3.LAMPORTS_PER_SOL);
        await connection.confirmTransaction({ signature, ...latestBlockHash });

        // Create Mint and Associated Token Accounts
        try {
            await spl.createMint(connection, alice, mintAuthority.publicKey, null, 0, mint);
        } catch (_) {
            console.log("Mint already exists");
        }
        aliceTokenAccount = await spl.createAssociatedTokenAccount(connection, alice, mint.publicKey, alice.publicKey);
        bobTokenAccount = await spl.createAssociatedTokenAccount(connection, alice, mint.publicKey, bob.publicKey);

        // Fund alice's token acc with tokens
        await spl.mintTo(connection, alice, mint.publicKey, aliceTokenAccount, mintAuthority, swapAmount.toNumber() * 10)
        await connection.confirmTransaction({ signature, ...latestBlockHash })

        console.log(
            `Account Information:
\tAlice:    ${alice.publicKey} \t Alice TokenAcc: \t${aliceTokenAccount}
\tBob:      ${bob.publicKey} \t Bob TokenAcc: \t${bobTokenAccount}
\tSwap Acc: ${swapAccount} \t Swap TokenAcc: \t\t${swapTokenAccount}
\tMint:     ${mint.publicKey}\t Mint Authority: \t${mintAuthority.publicKey}\n`
        );
    });

    async function aliceInitiate() {
        const signature = await program.methods
            .initiate(swapAmount, swapExpiresIn, bobTokenAccount, [...secretHash])
            .accounts({
                initiator: alice.publicKey,
                initiatorTokenAccount: aliceTokenAccount,
                mint: mint.publicKey,
            })
            .signers([alice])
            .rpc();
        await connection.confirmTransaction({signature, ...latestBlockHash});
        console.log(`\tInitiate: \t${signature}`);
    }

    it("Test initiation", async () => {
        const aliceBalanceBefore = (await connection.getTokenAccountBalance(aliceTokenAccount)).value.uiAmount;
        await aliceInitiate();
        const aliceBalance = (await connection.getTokenAccountBalance(aliceTokenAccount)).value.uiAmount;
        expect(aliceBalance - aliceBalanceBefore).to.equal(-swapAmount.toNumber());
    });

    it("Test redeem", async () => {
        // The previous testcase has initiated the swap
        const bobBalanceBefore = (await connection.getTokenAccountBalance(bobTokenAccount)).value.uiAmount;
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

        const bobBalance = (await connection.getTokenAccountBalance(bobTokenAccount)).value.uiAmount;
        expect(bobBalance - bobBalanceBefore).to.equal(swapAmount.toNumber());
    });

    it("Test refund", async () => {
        await aliceInitiate();  // Re-initiating for this test
        const expiryMs = swapExpiresIn.toNumber() * 400;
        console.log(`Awaiting timelock of ${expiryMs}ms for Refund`);
        await new Promise(r => setTimeout(r, expiryMs + 400));
        const aliceBalanceBefore = (await connection.getTokenAccountBalance(aliceTokenAccount)).value.uiAmount;
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

        const aliceBalance = (await connection.getTokenAccountBalance(aliceTokenAccount)).value.uiAmount;
        expect(aliceBalance - aliceBalanceBefore).to.equal(swapAmount.toNumber());
    });

    it("Test instant refund", async () => {
        await aliceInitiate();  // Re-initiating for this test
        const aliceBalanceBefore = (await connection.getTokenAccountBalance(aliceTokenAccount)).value.uiAmount;
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

        const aliceBalance = (await connection.getTokenAccountBalance(aliceTokenAccount)).value.uiAmount;
        expect(aliceBalance - aliceBalanceBefore).to.equal(swapAmount.toNumber());
    });
});
