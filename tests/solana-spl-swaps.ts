import {
  Program,
  AnchorProvider,
  workspace,
  web3,
  BN,
} from "@coral-xyz/anchor";
import * as spl from "@solana/spl-token";
import { expect } from "chai";
import crypto from "node:crypto";

import { SolanaSplSwaps } from "../target/types/solana_spl_swaps";

// Configure the client to use the local cluster.
const provider = AnchorProvider.env();
const connection = provider.connection;
const program = workspace.SolanaSplSwaps as Program<SolanaSplSwaps>;

describe("Testing one way swap between Alice and Bob", () => {
  const swapAmount = new BN(10);
  const swapExpiresIn = new BN(2); // 2 slots = 800 ms
  const secret: Buffer = crypto.randomBytes(32);
  const secretHash: Buffer = crypto
    .createHash("sha256")
    .update(secret)
    .digest();

  const mint = web3.Keypair.fromSeed(new Uint8Array(32).fill(33));
  const mintAuthority = web3.Keypair.fromSeed(new Uint8Array(32).fill(36));

  // Alice, the initiator
  const alice = new web3.Keypair();
  let aliceTokenAccount: web3.PublicKey;

  // Bob, the redeemer
  const bob = new web3.Keypair();
  let bobTokenAccount: web3.PublicKey;

  // Sponsors the PDA rent and transaction fees
  const sponsor = new web3.Keypair();

  const [swapData] = web3.PublicKey.findProgramAddressSync(
    [alice.publicKey.toBuffer(), secretHash],
    program.programId
  );
  const [tokenVault] = web3.PublicKey.findProgramAddressSync(
    [mint.publicKey.toBuffer()],
    program.programId
  );

  let latestBlockHash: web3.BlockhashWithExpiryBlockHeight;

  before(async () => {
    latestBlockHash = await connection.getLatestBlockhash();
    // Fund sponsor with 1 SOL
    const signature = await connection.requestAirdrop(
      sponsor.publicKey,
      web3.LAMPORTS_PER_SOL
    );
    await connection.confirmTransaction({ signature, ...latestBlockHash });

    // Create Mint and Associated Token Accounts
    try {
      await spl.createMint(
        connection,
        sponsor,
        mintAuthority.publicKey,
        null,
        0,
        mint
      );
    } catch (_) {
      console.log("Mint already exists");
    }
    aliceTokenAccount = await spl.createAssociatedTokenAccount(
      connection,
      sponsor,
      mint.publicKey,
      alice.publicKey
    );
    bobTokenAccount = await spl.createAssociatedTokenAccount(
      connection,
      sponsor,
      mint.publicKey,
      bob.publicKey
    );

    // Fund alice's token acc with tokens
    await spl.mintTo(
      connection,
      sponsor,
      mint.publicKey,
      aliceTokenAccount,
      mintAuthority,
      swapAmount.toNumber() * 10
    );
    await connection.confirmTransaction({ signature, ...latestBlockHash });

    console.log(
      `Account Information:
Alice     : ${alice.publicKey}\tAlice TokenAcc:\t${aliceTokenAccount}
Bob       : ${bob.publicKey}\tBob TokenAcc:\t${bobTokenAccount}
Swap Data : ${swapData}\tToken Vault:\t${tokenVault}
Mint      : ${mint.publicKey}\tMint Authority:\t${mintAuthority.publicKey}
Sponsor   : ${sponsor.publicKey}\n`
    );
  });

  async function aliceInitiate() {
    const signature = await program.methods
      .initiate(swapExpiresIn, bob.publicKey, [...secretHash], swapAmount)
      .accounts({
        initiator: alice.publicKey,
        initiatorTokenAccount: aliceTokenAccount,
        mint: mint.publicKey,
        sponsor: sponsor.publicKey,
      })
      .signers([alice, sponsor])
      .rpc();
    await connection.confirmTransaction({ signature, ...latestBlockHash });
    console.log(`\tInitiate: \t${signature}`);
  }

  it("Test initiation", async () => {
    const aliceBalanceBefore = (
      await connection.getTokenAccountBalance(aliceTokenAccount)
    ).value.uiAmount;
    await aliceInitiate();
    const aliceBalance = (
      await connection.getTokenAccountBalance(aliceTokenAccount)
    ).value.uiAmount;
    expect(aliceBalance - aliceBalanceBefore).to.equal(-swapAmount.toNumber());
  });

  it("Test redeem", async () => {
    // The previous testcase has initiated the swap
    const bobBalanceBefore = (
      await connection.getTokenAccountBalance(bobTokenAccount)
    ).value.uiAmount;
    const signature = await program.methods
      .redeem([...secret])
      .accounts({
        redeemerTokenAccount: bobTokenAccount,
        sponsor: sponsor.publicKey,
        swapData,
        tokenVault,
      })
      .rpc();
    await connection.confirmTransaction({ signature, ...latestBlockHash });
    console.log(`\tRedeem: \t${signature}`);

    const bobBalance = (
      await connection.getTokenAccountBalance(bobTokenAccount)
    ).value.uiAmount;
    expect(bobBalance - bobBalanceBefore).to.equal(swapAmount.toNumber());
  });

  it("Test refund", async () => {
    await aliceInitiate(); // Re-initiating for this test
    const expiryMs = swapExpiresIn.toNumber() * 400;
    console.log(`Awaiting timelock of ${expiryMs}ms for Refund`);
    await new Promise((r) => setTimeout(r, expiryMs + 400));
    const aliceBalanceBefore = (
      await connection.getTokenAccountBalance(aliceTokenAccount)
    ).value.uiAmount;
    const signature = await program.methods
      .refund()
      .accounts({
        initiatorTokenAccount: aliceTokenAccount,
        sponsor: sponsor.publicKey,
        swapData,
        tokenVault,
      })
      .rpc();
    await connection.confirmTransaction({ signature, ...latestBlockHash });
    console.log(`\tRefund: \t${signature}`);

    const aliceBalance = (
      await connection.getTokenAccountBalance(aliceTokenAccount)
    ).value.uiAmount;
    expect(aliceBalance - aliceBalanceBefore).to.equal(swapAmount.toNumber());
  });

  it("Test instant refund", async () => {
    await aliceInitiate(); // Re-initiating for this test
    const aliceBalanceBefore = (
      await connection.getTokenAccountBalance(aliceTokenAccount)
    ).value.uiAmount;
    const signature = await program.methods
      .instantRefund()
      .accounts({
        initiatorTokenAccount: aliceTokenAccount,
        redeemer: bob.publicKey,
        sponsor: sponsor.publicKey,
        swapData,
        tokenVault,
      })
      .signers([bob])
      .rpc();
    await connection.confirmTransaction({ signature, ...latestBlockHash });
    console.log(`\tInstant Refund:  ${signature}`);

    const aliceBalance = (
      await connection.getTokenAccountBalance(aliceTokenAccount)
    ).value.uiAmount;
    expect(aliceBalance - aliceBalanceBefore).to.equal(swapAmount.toNumber());
  });
});
