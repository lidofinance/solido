#!/usr/bin/env python3

"""
Deploy Solido and Serum on the testnet, bind the maintainer bot to the Solido.
"""

cluster = "https://api.testnet.solana.com"

import os

from util import (
    TestAccount,
    create_test_account,
    run,
    multisig,
    solido,
    solana,
    solana_program_deploy,
    get_solido_program_path,
    get_approve_and_execute,
    MAX_VALIDATION_COMMISSION_PERCENTAGE,
)


def maybe_from_file(path: str) -> str | None:
    try:
        return open(path).read().strip()
    except FileNotFoundError:
        return None


def write(content: str, /, to: str):
    open(to, "w").write(content)


def main():
    os.makedirs(".testnet-assets", exist_ok=True)
    os.environ["NETWORK"] = cluster

    solido_program_id = maybe_from_file(".testnet-assets/solido-program-id")
    if solido_program_id is None:
        print("\nUploading Solido program ...")
        solido_program_id = solana_program_deploy(
            get_solido_program_path() + "/lido.so"
        )
    else:
        print("\nUsing existing Solido program ...")
    print(f"> Solido program id is {solido_program_id}")
    write(solido_program_id, to=".testnet-assets/solido-program-id")

    multisig_program_id = maybe_from_file(".testnet-assets/multisig-program-id")
    if multisig_program_id is None:
        print("\nUploading Multisig program ...")
        multisig_program_id = solana_program_deploy(
            get_solido_program_path() + "/serum_multisig.so"
        )
    else:
        print("\nUsing existing Multisig program ...")
    print(f"> Multisig program id is {multisig_program_id}")
    write(multisig_program_id, to=".testnet-assets/multisig-program-id")

    maintainer = maybe_from_file(".testnet-assets/maintainer")
    if maintainer is None:
        print("\nGenerating maintainer keypair ...")
        maintainer = create_test_account(".testnet-assets/maintainer")
    else:
        print("\nUsing existing maintainer keypair ...")
        pubkey = run("solana-keygen", "pubkey", ".testnet-assets/maintainer").strip()
        maintainer = TestAccount(pubkey, ".testnet-assets/maintainer")
    print(f"> Maintainer is {maintainer}")

    owner = maybe_from_file(".testnet-assets/owner")
    if owner is None:
        print("\nGenerating stSOL owner keypair ...")
        owner = create_test_account(".testnet-assets/owner")
    else:
        print("\nUsing existing stSOL owner keypair ...")
        pubkey = run("solana-keygen", "pubkey", ".testnet-assets/owner").strip()
        owner = TestAccount(pubkey, ".testnet-assets/owner")
    print(f"> Owner is {owner}")

    signer = maybe_from_file(".testnet-assets/signer")
    if signer is None:
        print("\nGenerating signer keypair ...")
        signer = create_test_account(".testnet-assets/signer")
    else:
        print("\nUsing existing signer keypair ...")
        pubkey = run("solana-keygen", "pubkey", ".testnet-assets/signer").strip()
        signer = TestAccount(pubkey, ".testnet-assets/signer")
    print(f"> Signer is {signer}")

    multisig_instance = maybe_from_file(".testnet-assets/multisig-instance")
    if multisig_instance is None:
        print("\nCreating multisig instance ...")
        multisig_data = multisig(
            "create-multisig",
            "--multisig-program-id",
            multisig_program_id,
            "--threshold",
            "1",
            "--owners",
            maintainer.pubkey,
            keypair_path=signer.keypair_path,
        )
        multisig_instance = multisig_data["multisig_address"]
    else:
        print("\nUsing existing multisig instance ...")
    print(f"> Multisig instance is at {multisig_instance}")
    write(multisig_instance, to=".testnet-assets/multisig-instance")

    solido_address = maybe_from_file(".testnet-assets/solido-address")
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
            keypair_path=maintainer.keypair_path,
        )
        solido_address = result["solido_address"]
    else:
        print("\nUsing existing Solido instance ...")
    print(f"> Solido instance is at {solido_address}")
    write(solido_address, to=".testnet-assets/solido-address")

    solido_instance = solido(
        "show-solido",
        "--solido-program-id",
        solido_program_id,
        "--solido-address",
        solido_address,
    )

    solana(
        "program",
        "set-upgrade-authority",
        "--new-upgrade-authority",
        solido_instance["solido"]["manager"],
        solido_program_id,
    )

    approve_and_execute = get_approve_and_execute(
        multisig_program_id=multisig_program_id,
        multisig_instance=multisig_instance,
        signer_keypair_paths=[maintainer.keypair_path],
    )

    print("Adding maintainer ...")
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
        keypair_path=maintainer.keypair_path,
    )
    approve_and_execute(add_maintainer_tx["transaction_address"])


if __name__ == "__main__":
    main()
