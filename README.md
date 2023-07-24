# Openbook V2 Simultion

This project aims to simulate openbook v2 on solana cluster.

## Configure

Configure the required cluster using : 

```
# configure cluster for solana cli
solana config set --url CLUSTER_RPC

# deploy program
./configure/deploy-programs.sh

# install nodejs dependencies
yarn install

# configure openbook v2 markets users and fill orderbook
yarn ts-node configure/configure_openbook_v2.ts
```

Following are the arguments required by configure_openbook_v2.ts

```
  --url, -u <str>                                 - RPC url [optional]
  --authority, -a <str>                           - Authority with lots of SOLs [optional]
  --number-of-payers, -p <number>                 - Number of payers used for testing [optional]
  --payer-balance, -b <number>                    - Balance of payer in SOLs [optional]
  --number-of-mints, -m <number>                  - Number of mints [optional]
  --number-of-market-orders-per-user, -o <number> - Number of of market orders per user on each side [optional]
  --output-file, -o <str>                         - a string [optional]
```

## Run simulation

To run simulation.

```
cargo run -- -c configure/config.json
```

Other arguments : 
```
  -r, --rpc-url <RPC_URL>                                    [default: http://127.0.0.1:8899]
  -w, --ws-url <WS_URL>                                      [default: ws://127.0.0.1:8900]
  -f, --fanout-size <FANOUT_SIZE>                            tpu fanout [default: 16]
  -k, --identity <IDENTITY>                                  [default: ]
      --duration-in-seconds <DURATION_IN_SECONDS>            [default: 60]
      --quotes-per-seconds <QUOTES_PER_SECONDS>              [default: 1]
  -c, --simulation-configuration <SIMULATION_CONFIGURATION>  
  -t, --transaction-save-file <TRANSACTION_SAVE_FILE>        [default: ]
  -b, --block-data-save-file <BLOCK_DATA_SAVE_FILE>          [default: ]
  -a, --keeper-authority <KEEPER_AUTHORITY>                  [default: ]
      --transaction-retry-in-ms <TRANSACTION_RETRY_IN_MS>    [default: 10]
```