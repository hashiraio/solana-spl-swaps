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
  const timelock = new BN(2); // 2 slots = 800 ms
  const secret: Buffer = crypto.randomBytes(32);
  const secretHash: Buffer = crypto
    .createHash("sha256")
    .update(secret)
    .digest();
  const destinationData = crypto.randomBytes(256); // can be null

  const mint = web3.Keypair.fromSeed(new Uint8Array(32).fill(33));
  const mintAuthority = web3.Keypair.fromSeed(new Uint8Array(32).fill(36));

  // Alice, the initiator
  const alice = new web3.Keypair();
  let aliceTokenAccount: web3.PublicKey;

  // Bob, the redeemer
  const bob = new web3.Keypair();
  let bobTokenAccount: web3.PublicKey;

  // Sponsors the PDA rent and transaction fees
  const rentSponsor = new web3.Keypair();

  // Facilitates initiate on behalf
  const funder = new web3.Keypair();
  let funderTokenAccount: web3.PublicKey;

  const [swapData] = web3.PublicKey.findProgramAddressSync(
    [
      mint.publicKey.toBuffer(),
      bob.publicKey.toBuffer(),
      alice.publicKey.toBuffer(),
      secretHash,
      swapAmount.toArrayLike(Buffer, "le", 8),
      timelock.toArrayLike(Buffer, "le", 8),
    ],
    program.programId
  );
  const [tokenVault] = web3.PublicKey.findProgramAddressSync(
    [mint.publicKey.toBuffer()],
    program.programId
  );

  let latestBlockHash: web3.BlockhashWithExpiryBlockHeight;

  before(async () => {
    latestBlockHash = await connection.getLatestBlockhash();
    // Fund rent sponsor with 1 SOL
    const signature = await connection.requestAirdrop(
      rentSponsor.publicKey,
      web3.LAMPORTS_PER_SOL
    );
    await connection.confirmTransaction({ signature, ...latestBlockHash });

    // Create Mint and Associated Token Accounts
    try {
      await spl.createMint(
        connection,
        rentSponsor,
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
      rentSponsor,
      mint.publicKey,
      alice.publicKey
    );
    bobTokenAccount = await spl.createAssociatedTokenAccount(
      connection,
      rentSponsor,
      mint.publicKey,
      bob.publicKey
    );
    funderTokenAccount = await spl.createAssociatedTokenAccount(
      connection,
      rentSponsor,
      mint.publicKey,
      funder.publicKey
    );

    // Fund alice's token acc with tokens
    await spl.mintTo(
      connection,
      rentSponsor,
      mint.publicKey,
      aliceTokenAccount,
      mintAuthority,
      swapAmount.toNumber() * 10
    );
    await connection.confirmTransaction({ signature, ...latestBlockHash });

    // Fund funder's token acc with tokens
    await spl.mintTo(
      connection,
      rentSponsor,
      mint.publicKey,
      funderTokenAccount,
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
Sponsor   : ${rentSponsor.publicKey}\n`
    );
  });

  async function aliceInitiate() {
    const signature = await program.methods
      .initiate(
        bob.publicKey,
        alice.publicKey,
        [...secretHash],
        swapAmount,
        timelock,
        destinationData
      )
      .accounts({
        funder: alice.publicKey,
        funderTokenAccount: aliceTokenAccount,
        mint: mint.publicKey,
        rentSponsor: rentSponsor.publicKey,
      })
      .signers([alice, rentSponsor])
      .rpc();
    await connection.confirmTransaction({ signature, ...latestBlockHash });
    console.log(`\tInitiate: \t${signature}`);
  }

  it("Test initiate on behalf", async () => {
    const aliceBalanceBefore = (
      await connection.getTokenAccountBalance(aliceTokenAccount)
    ).value.uiAmount;
    const funderPreBalance = (
      await connection.getTokenAccountBalance(funderTokenAccount)
    ).value.uiAmount;

    const signature = await program.methods
      .initiate(
        bob.publicKey,
        alice.publicKey,
        [...secretHash],
        swapAmount,
        timelock,
        destinationData
      )
      .accounts({
        funder: funder.publicKey,
        funderTokenAccount,
        mint: mint.publicKey,
        rentSponsor: rentSponsor.publicKey,
      })
      .signers([funder, rentSponsor])
      .rpc();
    console.log(`\tFunder initiated on behalf of alice: \t${signature}`);

    const aliceBalance = (
      await connection.getTokenAccountBalance(aliceTokenAccount)
    ).value.uiAmount;
    expect(aliceBalance).to.equal(aliceBalanceBefore);

    const funderPostBalance = (
      await connection.getTokenAccountBalance(funderTokenAccount)
    ).value.uiAmount;
    expect(funderPostBalance).to.equal(
      funderPreBalance - swapAmount.toNumber()
    );
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
        rentSponsor: rentSponsor.publicKey,
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
    const timelockMs = timelock.toNumber() * 400;
    console.log(`Awaiting timelock of ${timelockMs}ms for Refund`);
    await new Promise((r) => setTimeout(r, timelockMs + 1000)); // Add an extra sec
    const aliceBalanceBefore = (
      await connection.getTokenAccountBalance(aliceTokenAccount)
    ).value.uiAmount;
    const signature = await program.methods
      .refund()
      .accounts({
        refundeeTokenAccount: aliceTokenAccount,
        rentSponsor: rentSponsor.publicKey,
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
        refundeeTokenAccount: aliceTokenAccount,
        redeemer: bob.publicKey,
        rentSponsor: rentSponsor.publicKey,
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
