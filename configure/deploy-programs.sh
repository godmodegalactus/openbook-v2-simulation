#! /bin/bash

SCRIPT_DIR=$( dirname -- "$0"; )

solana program deploy --program-id $SCRIPT_DIR/programs/openbook_v2-keypair.json $SCRIPT_DIR/programs/openbook_v2.so

solana program deploy --program-id $SCRIPT_DIR/programs/pyth_mock.json $SCRIPT_DIR/programs/pyth_mock.so

solana program deploy --program-id $SCRIPT_DIR/programs/spl_noop.json $SCRIPT_DIR/programs/spl_noop.so