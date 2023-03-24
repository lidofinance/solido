#!/bin/sh
set -euo pipefail

if [ -z "${NEW_AUTHORITY:-}" ]; then
    echo 'First set `NEW_AUTHORITY` to the new authority address'
    exit 2
fi

# Making sure the script is run from the root of the project
# and showing the addresses to be updated
jq . < solido_testnet_config.json

CLUSTER=`jq -r .cluster < solido_testnet_config.json`
SOLIDO_PROGRAM_ID=`jq -r .solido_program_id < solido_testnet_config.json`
MULTISIG_PROGRAM_ID=`jq -r .multisig_program_id < solido_testnet_config.json`
MULTISIG_ADDRESS=`jq -r .multisig_address < solido_testnet_config.json`

target/debug/solido --config=./solido_testnet_config.json \
                    --keypair-path=.testnet-assets/owner \
                    multisig \
                    set-upgrade-authority \
                    --multisig-address=${MULTISIG_ADDRESS} \
                    --multisig-program-id=${MULTISIG_PROGRAM_ID} \
                    --program-id=${SOLIDO_PROGRAM_ID} \
                    --new-authority=${NEW_AUTHORITY}

# now remember the transaction address it gives you, and run:
#
# target/debug/solido --config=./solido_testnet_config.json \
#                    --keypair-path=.testnet-assets/owner \
#                    multisig \
#                    approve \
#                    --transaction-address='...'
#
# target/debug/solido --config=./solido_testnet_config.json \
#                    --keypair-path=.testnet-assets/owner \
#                    multisig \
#                    execute-transaction \
#                    --transaction-address='...'
#
