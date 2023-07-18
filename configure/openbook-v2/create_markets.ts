import { Connection, Keypair, LAMPORTS_PER_SOL, PublicKey, sendAndConfirmTransaction } from "@solana/web3.js";
import IDL from "../programs/openbook_v2.json";
import { Program, web3, BN } from "@project-serum/anchor";
import { createAccount } from "../general/solana_utils";
import { MintUtils } from "../general/mint_utils";
import { I80F48, I80F48Dto, U64_MAX_BN } from "@blockworks-foundation/mango-v4";
import { OpenbookV2 } from "./openbook_v2";
import { TestProvider } from "../anchor_utils";

export interface Market {
  name: string;
  admin: number[];
  market_pk: PublicKey;
  oracle: PublicKey;
  asks: PublicKey;
  bids: PublicKey;
  event_queue: PublicKey;
  base_vault: PublicKey;
  quote_vault: PublicKey;
  base_mint: PublicKey;
  quote_mint: PublicKey;
  market_index: number;
  price: number,
}

function getRandomInt(max:  number) {
  return Math.floor(Math.random() * max) + 100;
}


export async function createMarket(
  program: Program<OpenbookV2>,
  anchorProvider: TestProvider,
  mintUtils: MintUtils,
  adminKp: Keypair,
  openbookProgramId: PublicKey,
  baseMint: PublicKey,
  quoteMint: PublicKey,
  index: number
): Promise<Market> {
  let [oracleId, _tmp] = PublicKey.findProgramAddressSync(
    [Buffer.from("StubOracle"), baseMint.toBytes()],
    openbookProgramId
  );

  let price = getRandomInt(1000);

  let sig = await anchorProvider.connection.requestAirdrop(adminKp.publicKey, 1000 * LAMPORTS_PER_SOL);
  await anchorProvider.connection.confirmTransaction(sig);

  await program.methods
    .stubOracleCreate({val: new BN(1)})
    .accounts({
      admin : adminKp.publicKey,
      payer: adminKp.publicKey,
      oracle: oracleId,
      mint: baseMint,
      systemProgram: web3.SystemProgram.programId,
    }).signers([adminKp]).rpc();

  await program.methods.stubOracleSet({
    val: new BN(price),
  }).accounts(
    {
      admin: adminKp.publicKey,
      oracle: oracleId,
    }
  ).signers([adminKp]).rpc();

  // bookside size = 123720
  let asks = await createAccount(
    anchorProvider.connection,
    anchorProvider.keypair,
    123720,
    openbookProgramId
  );
  let bids = await createAccount(
    anchorProvider.connection,
    anchorProvider.keypair,
    123720,
    openbookProgramId
  );
  let eventQueue = await createAccount(
    anchorProvider.connection,
    anchorProvider.keypair,
    101592,
    openbookProgramId
  );
  let marketIndex: BN = new BN(index);

  let [marketPk, _tmp2] = PublicKey.findProgramAddressSync(
    [Buffer.from("Market"), adminKp.publicKey.toBuffer(), marketIndex.toBuffer("le", 4)],
    openbookProgramId
  );

  let baseVault = await mintUtils.createTokenAccount(
    baseMint,
    anchorProvider.keypair,
    marketPk
  );
  let quoteVault = await mintUtils.createTokenAccount(
    quoteMint,
    anchorProvider.keypair,
    marketPk
  );
  let name = "index " + index.toString() + " wrt 0";

  await program.methods
    .createMarket(
      marketIndex,
      name,
      {
        confFilter: 0,
        maxStalenessSlots: 100,
      },
      new BN(1),
      new BN(1),
      new BN(0),
      new BN(0),
      new BN(0),
    )
    .accounts({
      market: marketPk,
      bids,
      asks,
      eventQueue,
      payer: adminKp.publicKey,
      baseVault,
      quoteVault,
      baseMint,
      quoteMint,
      systemProgram: web3.SystemProgram.programId,
      oracle: oracleId,
      collectFeeAdmin: adminKp.publicKey,
      openOrdersAdmin: null,
      closeMarketAdmin: null,
      consumeEventsAdmin: null,
    })
    .preInstructions([web3.ComputeBudgetProgram.setComputeUnitLimit({
      units: 10_000_000
    })])
    .signers([adminKp])
    .rpc();

  return {
    admin: Array.from(adminKp.secretKey),
    name,
    bids,
    asks,
    event_queue: eventQueue,
    base_mint: baseMint,
    base_vault: baseVault,
    market_index: index,
    market_pk: marketPk,
    oracle: oracleId,
    quote_mint: quoteMint,
    quote_vault: quoteVault,
    price,
  };
}
