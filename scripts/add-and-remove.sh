#!/bin/bash
set -eu
set -o pipefail

###############################################################################
#                              PREPARATIONS                                   #
###############################################################################

# On a fresh cluster,
# deploy the multisig program:
MULTISIG_PROGRAM_ID=`solana --url http://127.0.0.1:8899 --commitment confirmed program deploy --output json target/deploy/serum_multisig.so | jq -r .programId`

# Generate an owner:
solana-keygen new --no-bip39-passphrase --force --silent --outfile tests/.keys/owner.json
OWNER=`solana-keygen pubkey tests/.keys/owner.json`

# And another one:
solana-keygen new --no-bip39-passphrase --force --silent --outfile tests/.keys/other-owner.json
OTHER_OWNER=`solana-keygen pubkey tests/.keys/other-owner.json`

# Make sure they both have enough funds to operate on the network:
solana airdrop 10000 "${OWNER}"
solana airdrop 10000 "${OTHER_OWNER}"

# Create a multisig instance:
MULTISIG_OUT=`target/debug/solido --cluster http://127.0.0.1:8899 --output json multisig create-multisig --multisig-program-id ${MULTISIG_PROGRAM_ID} --threshold 1 --owners ${OWNER},${OTHER_OWNER}`
MULTISIG_ADDRESS=`echo "$MULTISIG_OUT" | jq -r .multisig_address`
MULTISIG_PROGRAM_DERIVED_ADDRESS=`echo "$MULTISIG_OUT" | jq -r .multisig_program_derived_address`

###############################################################################
#                        CREATING A TRANSACTION                               #
###############################################################################

TRANSACTION_ADDRESS=`target/debug/solido --output json --keypair-path tests/.keys/owner.json multisig propose-change-multisig \
  --threshold 1 \
  --owners "${OWNER}" \
  --multisig-program-id "${MULTISIG_PROGRAM_ID}" \
  --multisig-address "${MULTISIG_ADDRESS}" \
| jq -r .transaction_address
`

###############################################################################
#                        APPROVING THE TRANSACTION                            #
###############################################################################

# Now that we have a transaction, let's approve it:
target/debug/solido --output text --keypair-path tests/.keys/owner.json multisig approve \
  --multisig-program-id "${MULTISIG_PROGRAM_ID}" \
  --multisig-address "${MULTISIG_ADDRESS}" \
  --transaction-address ${TRANSACTION_ADDRESS}

###############################################################################
#                        EXECUTING THE TRANSACTION                            #
###############################################################################

# And off we go:
target/debug/solido --output text --keypair-path tests/.keys/owner.json multisig execute-transaction \
  --multisig-program-id "${MULTISIG_PROGRAM_ID}" \
  --multisig-address "${MULTISIG_ADDRESS}" \
  --transaction-address ${TRANSACTION_ADDRESS}
