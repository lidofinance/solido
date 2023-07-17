#!/usr/bin/env python3

# SPDX-FileCopyrightText: 2021 Chorus One AG
# SPDX-License-Identifier: GPL-3.0

"""
Set up a Solido instance on a local testnet, and print its details. This is
useful when testing the maintenance daemon locally.
"""

import json
import os
from typing import Optional, Any

import util


class Instance:
    def __init__(self) -> None:
        print('\nUploading Solido program ...')
        self.solido_program_id = util.solana_program_deploy(
            util.get_solido_program_path() + '/lido.so'
        )
        print(f'> Solido program id is {self.solido_program_id}')

        print('\nUploading Multisig program ...')
        self.multisig_program_id = util.solana_program_deploy(
            util.get_solido_program_path() + '/serum_multisig.so'
        )
        print(f'> Multisig program id is {self.multisig_program_id}')

        os.makedirs('tests/.keys', exist_ok=True)
        self.maintainer = util.create_test_account('tests/.keys/maintainer.json')
        st_sol_accounts_owner = util.create_test_account(
            'tests/.keys/st-sol-accounts-owner.json'
        )

        print('\nCreating new multisig ...')
        multisig_data = util.multisig(
            'create-multisig',
            '--multisig-program-id',
            self.multisig_program_id,
            '--threshold',
            '1',
            '--owners',
            self.maintainer.pubkey,
        )
        self.multisig_instance = multisig_data['multisig_address']
        print(f'> Created instance at {self.multisig_instance}')

        print('\nCreating Solido instance ...')
        result = util.solido(
            'create-solido',
            '--multisig-program-id',
            self.multisig_program_id,
            '--solido-program-id',
            self.solido_program_id,
            '--max-validators',
            '9',
            '--max-maintainers',
            '3',
            '--max-commission',
            str(util.MAX_VALIDATION_COMMISSION_PERCENTAGE),
            '--min-block-production-rate',
            '0',
            '--min-vote-success-rate',
            '0',
            '--min-uptime',
            '0',
            '--treasury-fee-share',
            '5',
            '--developer-fee-share',
            '2',
            '--st-sol-appreciation-share',
            '93',
            '--treasury-account-owner',
            st_sol_accounts_owner.pubkey,
            '--developer-account-owner',
            st_sol_accounts_owner.pubkey,
            '--multisig-address',
            self.multisig_instance,
            keypair_path=self.maintainer.keypair_path,
        )

        self.solido_address = result['solido_address']
        self.treasury_account = result['treasury_account']
        self.developer_account = result['developer_account']
        self.st_sol_mint_account = result['st_sol_mint_address']
        self.validator_list_address = result['validator_list_address']
        self.maintainer_list_address = result['maintainer_list_address']

        print(f'> Created instance at {self.solido_address}')

        solido_instance = self.pull_solido()
        util.solana(
            'program',
            'set-upgrade-authority',
            '--new-upgrade-authority',
            solido_instance['solido']['manager'],
            self.solido_program_id,
        )

        self.approve_and_execute = util.get_approve_and_execute(
            multisig_program_id=self.multisig_program_id,
            multisig_instance=self.multisig_instance,
            signer_keypair_paths=[self.maintainer.keypair_path],
        )

        # For the first validator, add the test validator itself, so we include a
        # validator that is actually voting, and earning rewards.
        current_validators = json.loads(util.solana('validators', '--output', 'json'))

        # If we're running on localhost, change the commission
        if util.get_network() == 'http://127.0.0.1:8899':
            solido_instance = self.pull_solido()
            print(
                '> Changing validator\'s commission to {}% ...'.format(
                    util.MAX_VALIDATION_COMMISSION_PERCENTAGE
                )
            )
            validator = current_validators['validators'][0]
            validator['commission'] = str(util.MAX_VALIDATION_COMMISSION_PERCENTAGE)
            util.solana(
                'vote-update-commission',
                validator['voteAccountPubkey'],
                str(util.MAX_VALIDATION_COMMISSION_PERCENTAGE),
                './test-ledger/vote-account-keypair.json',
            )
            util.solana(
                'validator-info',
                'publish',
                '--keypair',
                './test-ledger/validator-keypair.json',
                "solana-test-validator",
            )

        # Allow only validators that are voting, have 100% commission, and have their
        # withdrawer set to Solido's rewards withdraw authority. On a local testnet,
        # this will only contain the test validator, but on devnet or testnet, there can
        # be more validators.
        active_validators = [
            v
            for v in current_validators['validators']
            if (not v['delinquent'])
            and v['commission'] == str(util.MAX_VALIDATION_COMMISSION_PERCENTAGE)
        ]

        # Add up to 5 of the active validators. Locally there will only be one, but on
        # the devnet or testnet there can be more, and we don't want to add *all* of them.
        validators = [
            self.add_validator(i, vote_account=v['voteAccountPubkey'])
            for (i, v) in enumerate(active_validators[:5])
        ]

        # Create two validators of our own, so we have a more interesting stake
        # distribution. These validators are not running, so they will not earn
        # rewards.
        # validators.extend(
        #     self.add_validator(i, vote_account=None)
        #     for i in range(len(validators), len(validators) + 1)
        # )

        print('Adding maintainer ...')
        transaction_result = util.solido(
            'add-maintainer',
            '--multisig-program-id',
            self.multisig_program_id,
            '--solido-program-id',
            self.solido_program_id,
            '--solido-address',
            self.solido_address,
            '--maintainer-address',
            self.maintainer.pubkey,
            '--multisig-address',
            self.multisig_instance,
            keypair_path=self.maintainer.keypair_path,
        )
        self.approve_and_execute(transaction_result['transaction_address'])

        output = {
            "cluster": util.get_network(),

            "max_commission": "5",
            "treasury_fee_share": "1",
            "developer_fee_share": "1",
            "max_validators": "256",
            "max_maintainers": "16",
            "st_sol_appreciation_share": "1",

            "treasury_account_owner": util.solana('address').strip(),
            "developer_account_owner": util.solana('address').strip(),

            "multisig_program_id": self.multisig_program_id,
            "multisig_address": self.multisig_instance,
            "solido_program_id": self.solido_program_id,
            "solido_address": self.solido_address,
            "st_sol_mint": self.st_sol_mint_account,
        }
        print("Config file is ../solido_test.json")
        with open('../solido_test.json', 'w') as outfile:
            json.dump(output, outfile, indent=4)

        for i, vote_account in enumerate(validators):
            print(f'  Validator {i} vote account: {vote_account}')

        print('\nMaintenance command line:')
        print(
            ' ',
            ' '.join(
                [
                    'solido',
                    '--keypair-path',
                    self.maintainer.keypair_path,
                    '--config',
                    '../solido_test.json',
                    'run-maintainer',
                    '--max-poll-interval-seconds',
                    '10',
                ]
            ),
        )

    def pull_solido(self) -> Any:
        return util.solido(
            'show-solido',
            '--solido-program-id',
            self.solido_program_id,
            '--solido-address',
            self.solido_address,
        )

    def add_validator(self, index: int, vote_account: Optional[str]) -> str:
        """
        Add a validator to the instance, create the right accounts for it. The vote
        account can be a pre-existing one, but if it is not provided, we will create
        one. Returns the vote account address.
        """
        print(f'\nCreating validator {index} ...')

        if vote_account is None:
            validator = util.create_test_account(
                f'tests/.keys/validator-{index}-account.json'
            )
            validator_vote_account, _ = util.create_vote_account(
                f'tests/.keys/validator-{index}-vote-account.json',
                validator.keypair_path,
                f'tests/.keys/validator-{index}-withdraw-account.json',
                util.MAX_VALIDATION_COMMISSION_PERCENTAGE,
            )
            vote_account = validator_vote_account.pubkey

        print(f'> Validator vote account:        {vote_account}')

        print('Adding validator ...')
        transaction_result = util.solido(
            'add-validator',
            '--multisig-program-id',
            self.multisig_program_id,
            '--solido-program-id',
            self.solido_program_id,
            '--solido-address',
            self.solido_address,
            '--validator-vote-account',
            vote_account,
            '--multisig-address',
            self.multisig_instance,
            keypair_path=self.maintainer.keypair_path,
        )
        self.approve_and_execute(transaction_result['transaction_address'])
        return vote_account


if __name__ == "__main__":
    Instance()
