#!/usr/bin/env python3

"""
This script is a helper to deploy a Solido instance on the testnet.
It will create and fund a maintainer account, a multisig owner account, and a multisig account.
It will also deploy the Solido program, the Multisig program, and create a Solido instance.

In order to run this script, you will need to have the Solana CLI and the Lido CLI installed.
You will also need to have the Rust toolchain installed, and the Lido CLI must be compiled.
See the Lido repository for more information.

The script will populate the `.testnet-keys` directory with the generated keypairs
if they are not already present there. It will also create a file named `solido_testnet_config.json`.

The script will also need the SOLIDO_PROGRAM_PATH environment variable to point to the
directory containing the compiled Solido program.

The script will also need the NETWORK environment variable to be set to the network on which
the Solido instance will be deployed. The default is the devnet. You can also set it to
the testnet by setting the NETWORK environment variable to "https://api.testnet.solana.com".
"""

import os
import json

from util import (
    TestAccount,
    create_test_account,
    run,
    multisig,
    maybe_from_file,
    solido,
    solana,
    solana_program_deploy,
    get_solido_program_path,
    get_approve_and_execute,
    validators,
    MAX_VALIDATION_COMMISSION_PERCENTAGE,
)


well_known_config_location = "solido_testnet_config.json"


def main():
    config = maybe_from_file(well_known_config_location)
    if config is None:
        config = {
            "cluster": os.getenv("NETWORK") or "https://api.devnet.solana.com",
        }
    else:
        config = json.loads(config)

    # Hacky, but we don't have time to rework `utils` right now.
    # Propagating `cluster` to the environment so that the whole `utils` see it.
    os.environ["NETWORK"] = config["cluster"]

    os.makedirs(".testnet-keys", exist_ok=True)

    ### Checking the keys are present and creating them if not
    maintainer = maybe_from_file(".testnet-keys/maintainer")
    if maintainer is None:
        print("\nGenerating maintainer keypair ...")
        maintainer = create_test_account(".testnet-keys/maintainer")
    else:
        print("\nUsing existing maintainer keypair ...")
        pubkey = run("solana-keygen", "pubkey", ".testnet-keys/maintainer").strip()
        maintainer = TestAccount(pubkey, ".testnet-assets/maintainer")
    print(f"> Maintainer is {maintainer}")

    owner = maybe_from_file(".testnet-keys/owner")
    if owner is None:
        print("\nGenerating a multisig owner keypair ...")
        owner = create_test_account(".testnet-keys/owner")
    else:
        print("\nUsing existing multisig owner keypair ...")
        pubkey = run("solana-keygen", "pubkey", ".testnet-keys/owner").strip()
        owner = TestAccount(pubkey, ".testnet-keys/owner")
    print(f"> Owner is {owner}")

    ### Solido
    solido_program_id = config["solido_program_id"]
    if solido_program_id is None:
        print("\nUploading Solido program ...")
        result = solana(
            "--keypair=.testnet-keys/owner",
            "program",
            "deploy",
            "--output=json",
            get_solido_program_path() + "/lido.so",
        )
        program_id: str = json.loads(result)["programId"]
        solido_program_id = program_id
    else:
        print("\nUsing existing Solido program ...")
    print(f"> Solido program id is {solido_program_id}")
    config["solido_program_id"] = solido_program_id
    with open(well_known_config_location, "w") as f:
        json.dump(config, f)

    ### Multisig
    multisig_program_id = config["multisig_program_id"]
    if multisig_program_id is None:
        print("\nUploading Multisig program ...")
        multisig_program_id = solana_program_deploy(
            get_solido_program_path() + "/serum_multisig.so"
        )
    else:
        print("\nUsing existing Multisig program ...")
    print(f"> Multisig program id is {multisig_program_id}")
    config["multisig_program_id"] = multisig_program_id
    with open(well_known_config_location, "w") as f:
        json.dump(config, f)

    multisig_instance = config["multisig_address"]
    if multisig_instance is None:
        print("\nCreating multisig instance ...")
        multisig_data = multisig(
            "create-multisig",
            "--multisig-program-id",
            multisig_program_id,
            "--threshold",
            "1",
            "--owners",
            owner.pubkey,
            keypair_path=owner.keypair_path,
        )
        multisig_instance = multisig_data["multisig_address"]
    else:
        print("\nUsing existing multisig instance ...")
    print(f"> Multisig instance is at {multisig_instance}")
    config["multisig_address"] = multisig_instance
    with open(well_known_config_location, "w") as f:
        json.dump(config, f)

    solido_address = config["solido_address"]
    if solido_address is None:
        print("\nCreating Solido instance ...")
        result = solido(
            "create-solido",
            "--multisig-program-id",
            multisig_program_id,
            "--solido-program-id",
            solido_program_id,
            "--max-validators",
            "9",
            "--max-maintainers",
            "3",
            "--max-commission-percentage",
            str(MAX_VALIDATION_COMMISSION_PERCENTAGE),
            "--treasury-fee-share",
            "5",
            "--developer-fee-share",
            "2",
            "--st-sol-appreciation-share",
            "93",
            "--treasury-account-owner",
            owner.pubkey,
            "--developer-account-owner",
            owner.pubkey,
            "--multisig-address",
            multisig_instance,
            keypair_path=owner.keypair_path,
        )
        solido_address = result["solido_address"]
    else:
        print("\nUsing existing Solido instance ...")
    print(f"> Solido instance is at {solido_address}")
    config["solido_address"] = solido_address
    with open(well_known_config_location, "w") as f:
        json.dump(config, f)

    solido_instance = solido(
        "show-solido",
        "--solido-program-id",
        solido_program_id,
        "--solido-address",
        solido_address,
    )

    solana(
        "--keypair=.testnet-keys/owner",
        "program",
        "set-upgrade-authority",
        "--new-upgrade-authority",
        solido_instance["solido"]["manager"],
        solido_program_id,
    )

    approve_and_execute = get_approve_and_execute(
        multisig_program_id=multisig_program_id,
        multisig_instance=multisig_instance,
        signer_keypair_paths=[owner.keypair_path],
    )

    all_validators = validators()
    active_validators = [
        v
        for v in all_validators
        if not v.delinquent and v.commission == MAX_VALIDATION_COMMISSION_PERCENTAGE
    ]
    for v in active_validators[:2]:
        add_validator_tx = solido(
            "add-validator",
            "--multisig-program-id",
            multisig_program_id,
            "--solido-program-id",
            solido_program_id,
            "--solido-address",
            solido_address,
            "--validator-vote-account",
            v.vote_account_pubkey,
            "--multisig-address",
            multisig_instance,
            keypair_path=owner.keypair_path,
        )
        approve_and_execute(add_validator_tx["transaction_address"])

    print("\nAdding maintainer ...")
    add_maintainer_tx = solido(
        "add-maintainer",
        "--multisig-program-id",
        multisig_program_id,
        "--solido-program-id",
        solido_program_id,
        "--solido-address",
        solido_address,
        "--maintainer-address",
        maintainer.pubkey,
        "--multisig-address",
        multisig_instance,
        keypair_path=owner.keypair_path,
    )
    approve_and_execute(add_maintainer_tx["transaction_address"])

    output = {
        "cluster": config["cluster"],
        "multisig_program_id": multisig_program_id,
        "multisig_address": multisig_instance,
        "solido_program_id": solido_program_id,
        "solido_address": solido_address,
        "st_sol_mint": "(tbd)",  ##
    }
    print(f"> Config file is `./{well_known_config_location}`")
    with open(well_known_config_location, "w") as outfile:
        json.dump(output, outfile, indent=4)


if __name__ == "__main__":
    main()
