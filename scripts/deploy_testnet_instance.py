#!/usr/bin/env python3

"""
This script does the following:
* In directory `.testnet-keys`,
  - Generates a maintainer keypair if it doesn't exist yet
  - Generates a multisig owner keypair if it doesn't exist yet
* Within the network specified by the `NETWORK` environment variable or by the config file `solido_testnet_config.json`,
  - Uploads the Solido program if its address is not specified in the config
  - Uploads the Multisig program if its address is not specified in the config
  - Creates a Solido instance if its address is not specified in the config
  - Adds certain validators to the Solido instance
  - Adds the maintainer as a maintainer of the Solido instance
If successful, the script will update the config file `solido_testnet_config.json` with the new addresses.
"""

import os
import json

import util

# The config is expected to be a JSON file at the root of the project.
config_path = "solido_testnet_config.json"


def main():
    config = util.maybe_from_file(config_path)
    config = json.loads(config) if config is not None else {}
    config["cluster"] = (
        config.get("cluster", os.getenv("NETWORK")) or "https://api.devnet.solana.com"
    )

    # Propagating `cluster` to the environment so that the whole `utils` see it.
    os.environ["NETWORK"] = config["cluster"]

    os.makedirs(".testnet-keys", exist_ok=True)

    # Checking the keys are present and creating them if not
    maintainer = util.maybe_from_file(".testnet-keys/maintainer")
    if maintainer is None:
        print("\nGenerating maintainer keypair ...")
        maintainer = util.create_test_account(".testnet-keys/maintainer")
    else:
        print("\nUsing existing maintainer keypair ...")
        pubkey = util.run("solana-keygen", "pubkey", ".testnet-keys/maintainer").strip()
        maintainer = util.TestAccount(pubkey, ".testnet-keys/maintainer")
    print(f"> Maintainer is {maintainer}")

    multisig_owner = util.maybe_from_file(".testnet-keys/multisig-owner")
    if multisig_owner is None:
        print("\nGenerating a multisig owner keypair ...")
        multisig_owner = util.create_test_account(".testnet-keys/multisig-owner")
    else:
        print("\nUsing existing multisig owner keypair ...")
        pubkey = util.run(
            "solana-keygen", "pubkey", ".testnet-keys/multisig-owner"
        ).strip()
        multisig_owner = util.TestAccount(pubkey, ".testnet-keys/multisig-owner")
    print(f"> Owner is {multisig_owner}")

    # Deploying the Solido program
    solido_program_id = config.get("solido_program_id")
    if solido_program_id is None:
        print("\nUploading Solido program ...")
        result = util.solana(
            "--keypair=.testnet-keys/multisig-owner",
            "program",
            "deploy",
            "--output=json",
            util.get_solido_program_path() + "/lido.so",
        )
        program_id: str = json.loads(result)["programId"]
        solido_program_id = program_id
    else:
        print("\nUsing existing Solido program ...")
    print(f"> Solido program id is {solido_program_id}")
    config["solido_program_id"] = solido_program_id
    with open(config_path, "w") as f:
        json.dump(config, f)

    # Deploying the Multisig program
    multisig_program_id = config.get("multisig_program_id")
    if multisig_program_id is None:
        print("\nUploading Multisig program ...")
        result = util.solana(
            "--keypair=.testnet-keys/multisig-owner",
            "program",
            "deploy",
            "--output=json",
            util.get_solido_program_path() + "/serum_multisig.so",
        )
        program_id: str = json.loads(result)["programId"]
        multisig_program_id = program_id
    else:
        print("\nUsing existing Multisig program ...")
    print(f"> Multisig program id is {multisig_program_id}")
    config["multisig_program_id"] = multisig_program_id
    with open(config_path, "w") as f:
        json.dump(config, f)

    multisig_instance = config.get("multisig_address")
    if multisig_instance is None:
        print("\nCreating multisig instance ...")
        multisig_data = util.multisig(
            "create-multisig",
            "--multisig-program-id",
            multisig_program_id,
            "--threshold",
            "1",
            "--owners",
            multisig_owner.pubkey,
            keypair_path=multisig_owner.keypair_path,
        )
        multisig_instance = multisig_data["multisig_address"]
    else:
        print("\nUsing existing multisig instance ...")
    print(f"> Multisig instance is at {multisig_instance}")
    config["multisig_address"] = multisig_instance
    with open(config_path, "w") as f:
        json.dump(config, f)

    solido_address = config.get("solido_address")
    if solido_address is None:
        print("\nCreating Solido instance ...")
        result = util.solido(
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
            str(util.MAX_VALIDATION_COMMISSION_PERCENTAGE),
            "--treasury-fee-share",
            "5",
            "--developer-fee-share",
            "2",
            "--st-sol-appreciation-share",
            "93",
            "--treasury-account-owner",
            multisig_owner.pubkey,
            "--developer-account-owner",
            multisig_owner.pubkey,
            "--multisig-address",
            multisig_instance,
            keypair_path=multisig_owner.keypair_path,
        )
        solido_address = result["solido_address"]

        solido_instance = util.solido(
            "show-solido",
            "--solido-program-id",
            solido_program_id,
            "--solido-address",
            solido_address,
        )

        util.solana(
            "--keypair=.testnet-keys/multisig-owner",
            "program",
            "set-upgrade-authority",
            "--new-upgrade-authority",
            solido_instance["solido"]["manager"],
            solido_program_id,
        )

        approve_and_execute = util.get_approve_and_execute(
            multisig_program_id=multisig_program_id,
            multisig_instance=multisig_instance,
            signer_keypair_paths=[multisig_owner.keypair_path],
        )

        all_validators = util.validators()
        active_validators = [
            v
            for v in all_validators
            if not v.delinquent
            and v.commission == util.MAX_VALIDATION_COMMISSION_PERCENTAGE
        ]
        for v in active_validators[:2]:
            add_validator_tx = util.solido(
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
                keypair_path=multisig_owner.keypair_path,
            )
            approve_and_execute(add_validator_tx["transaction_address"])

        print("\nAdding maintainer ...")
        add_maintainer_tx = util.solido(
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
            keypair_path=multisig_owner.keypair_path,
        )
        approve_and_execute(add_maintainer_tx["transaction_address"])
    else:
        print("\nUsing existing Solido instance ...")
    print(f"> Solido instance is at {solido_address}")
    config["solido_address"] = solido_address
    with open(config_path, "w") as f:
        json.dump(config, f)

    solido_instance = util.solido(
        "show-solido",
        "--solido-program-id",
        solido_program_id,
        "--solido-address",
        solido_address,
    )

    output = {
        "cluster": config["cluster"],
        "multisig_program_id": multisig_program_id,
        "multisig_address": multisig_instance,
        "solido_program_id": solido_program_id,
        "solido_address": solido_address,
        "st_sol_mint": solido_instance["solido"]["st_sol_mint"],
    }
    print(f"\nWritten back config file at `./{config_path}`")
    with open(config_path, "w") as outfile:
        json.dump(output, outfile, indent=4)


if __name__ == "__main__":
    main()
