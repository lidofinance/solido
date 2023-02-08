#!/usr/bin/env python3

"""
This script has multiple options to to interact with Solido
"""

import sys
import os.path
from typing import Any, Dict, Set, List
from util import solido, solana, run  # type: ignore

Sample: Dict[str, Any] = {
    'solido_instance': '49Yi1TKkNyYjPAFdR9LBvoHcUjuPX4Df5T5yv39w2XTn',  # "solido_address": "49Yi1TKkNyYjPAFdR9LBvoHcUjuPX4Df5T5yv39w2XTn",
    'program_to_upgrade': 'CrX7kMhLC3cSsXJdT7JDgqrRVWGnUpX3gfEfxxU2NVLi',  # solido_config.json : solido_program_id
    'manager': 'GQ3QPrB1RHPRr4Reen772WrMZkHcFM4DL5q44x1BBTFm', # manager
    'buffer_address': '46Kdub5aehm8RpFtSvnaTWxYR2WMCgAkma7fj61vaRiT',  # buffer adres account
    'validator_list': 'GL9kqRNUTUosW3RsDoXHCuXUZn73SgQQmBvtp1ng2co4',
    'maintainer_list': '5dvtr16i34hwXuCtTNHXXJ5ojeidVPXbceN9pXxrE8bn',
    'developer_account': '5Y5LVTXbtMYsibjp9uQMmCyZbtSru8zktuxGPV9eHu3m',
    'reward_distribution': {
        'treasury_fee': 4,
        'developer_fee': 1,
        'st_sol_appreciation': 95,
    },
    'max_validators': 6700,
    'max_maintainers': 5000,
    'max_commission_percentage': 5,
}


ValidatorVoteAccSet = set()
VerificationStatus = True
ValidatorSetV1 = set()
ValidatorSetV2: Set[str] = { # set() Filled with updated vote accounts
    "9GJmEHGom9eWo4np4L5vC6b6ri1Df2xN8KFoWixvD1Bs",
    "DdCNGDpP7qMgoAy6paFzhhak2EeyCZcgjH7ak5u5v28m",
    "2NxEEbhqqj1Qptq5LXLbDTP5tLa9f7PqkU8zNgxbGU9P",
    "4PsiLMyoUQ7QRn1FFiFCvej4hsUTFzfvJnyN4bj1tmSN",
    "8jxSHbS4qAnh5yueFp4D9ABXubKqMwXqF3HtdzQGuphp",
    "BxFf75Vtzro2Hy3coFHKxFMZo5au8W7J8BmLC3gCMotU",
    "2vZd7mdsiDiXvGUq1ASNfkYYjMJ83yYXKHA3zfmKHc4g",
    "FCvNkHa4U3yh7AXWGGL2jWLWiSRouR8EtzY5WVTHKTHa",
    "7DrGM5rSgw8iCnXNxgjfmy4GFy6PuKu3gsujT5TjcDaA",
    "4MpRU9fDDSQNNTeb4v5DPZZTKupYancGksH679AKLBnt",
    "G11K4toVD1rk4ri7eziJyYENZTXb8h7q59gzaoE3BCHX",
    "BH7asDZbKkTmT3UWiNfmMVRgQEEpXoVThGPmQfgWwDhg",
    "7PmWxxiTneGteGxEYvzj5pGDVMQ4nuN9DfUypEXmaA8o",
    "EogKVYgic8LKAuV1kR9nRqJaS5zpwCvSMfqoehzmAMpK",
    "6F5xdRXh2W3B2vhte12VG79JVUkUSLYrHydGX1SAadfZ",
    "81LF3sFyx9aANNhZPTyPEULKHV1mTqd3qho7ZLQghNJL",
    "9J7Hvujf8LZiKBaXGmA1XwYszfVenieTdta1imwoC3QD",
    "Fw34MoMfRrAUPePPbfKAH9eQDizYodVXWV4fXSdjJwMa",
    "C5Tof5G3wNY1qg2z9HMfVrpQmvjUiaGj5SuYTYWeWWsm",
    "SFund7s2YPS7iCu7W2TobbuQEpVEAv9ZU7zHKiN1Gow"
}
SolidoVersion = -1
SolidoState = "Unknown state"
TransOrder: List[str] = list()


def printSolution(flag: bool) -> str:
    if flag:
        return " [OK]\n"
    else:
        global VerificationStatus
        VerificationStatus = False
        return " [BAD]\n"


def checkSolidoState(state: str) -> bool:
    return SolidoState == state


def checkVoteInV1Set(address: str) -> bool:
    return address in ValidatorSetV1


def checkVoteInV2Set(address: str) -> bool:
    return address in ValidatorSetV2


def checkVoteUnic(address: str) -> bool:
    if address not in ValidatorVoteAccSet:
        ValidatorVoteAccSet.add(address)
        return True
    else:
        return False


def ValidateSolidoState(state: str) -> str:
    return ": Solido state " + state + printSolution(SolidoState == state)


def ValidateField(dataDict: Any, key: str) -> str:
    value = dataDict.get(key)
    retbuf = key + " " + str(value)
    if key in dataDict.keys():
        retbuf += printSolution(value == Sample.get(key))
    else:
        retbuf += printSolution(False)
    return retbuf


def ValidateRewardField(dataDict: Any, key: str) -> str:
    value = dataDict.get(key)
    retbuf = key + " " + str(value)
    if key in dataDict.keys():
        reward_distribution = Sample.get('reward_distribution')
        if reward_distribution is not None:
            sampleValue = reward_distribution.get(key)
            if sampleValue != None:
                retbuf += printSolution(value == sampleValue)
                return retbuf

    retbuf += printSolution(False)
    return retbuf


def ValidateDeactivateV1VoteAccount(dataDict: Any, key: str) -> str:
    value = dataDict.get(key)
    retbuf = key + " " + str(value)
    if key in dataDict.keys():
        retbuf += printSolution(checkVoteUnic(value) and checkVoteInV1Set(value))
    else:
        retbuf += printSolution(False)
    return retbuf


def ValidateAddV2VoteAccount(dataDict: Any, key: str) -> str:
    value = dataDict.get(key)
    retbuf = key + " " + str(value)
    if key in dataDict.keys():
        retbuf += printSolution(checkVoteUnic(value) and checkVoteInV2Set(value))
    else:
        retbuf += printSolution(False)
    return retbuf


def ValidateTransOrder(trans):
    retbuf = "Transaction order "
    if trans == "BpfLoaderUpgrade":
        retbuf += "BpfLoaderUpgrade"
        retbuf += printSolution(len(TransOrder) == 0)
    elif trans == "MigrateStateToV2":
        retbuf += "MigrateStateToV2"
        retbuf += printSolution(
            len(TransOrder) == 1 and TransOrder[0] == "BpfLoaderUpgrade"
        )
    else:
        retbuf += printSolution(False)
    return retbuf


def verify_solido_state() -> None:
    # get solido state
    json_data = solido('--config', os.getenv("SOLIDO_CONFIG"), 'show-solido')

    # parse solido state
    l1_keys = json_data.get('solido')
    global SolidoVersion
    SolidoVersion = l1_keys.get('lido_version')
    validators = l1_keys.get('validators')
    if validators != None:
        for validator in validators.get('entries'):
            vote_acc = validator.get('pubkey')
            if validator.get('entry').get('active') == True:
                ValidatorSetV1.add(vote_acc)

    # detect current state
    global SolidoState
    if SolidoVersion == 0:
        if len(ValidatorSetV1) == 21:
            SolidoState = "Deactivate validators"
        elif len(ValidatorSetV1) == 0:
            SolidoState = "Upgrade program"
        else:
            SolidoState = "Unknown state - solido version = "
            SolidoState += str(SolidoVersion)
            SolidoState += " active validators count = "
            SolidoState += str(len(ValidatorSetV1))
    elif SolidoVersion == 1 and len(ValidatorSetV1) == 0:
        SolidoState = "Add validators"
    else:
        SolidoState = "Unknown state - solido version = "
        SolidoState += str(SolidoVersion)
        SolidoState += " active validators count = "
        SolidoState += str(len(ValidatorSetV1))

    # output result
    print("\nCurrent migration state: " + SolidoState)


def verify_transaction_data(json_data: Any) -> bool:
    l1_keys = json_data['parsed_instruction']
    output_buf = ""
    global VerificationStatus
    VerificationStatus = True
    if 'SolidoInstruction' in l1_keys.keys():
        output_buf += "SolidoInstruction "
        l2_data = l1_keys['SolidoInstruction']
        if 'DeactivateValidator' in l2_data.keys():
            output_buf += "DeactivateValidator"
            output_buf += ValidateSolidoState("Deactivate validators")
            trans_data = l2_data['DeactivateValidator']
            output_buf += ValidateField(trans_data, 'solido_instance')
            output_buf += ValidateField(trans_data, 'manager')
            output_buf += ValidateDeactivateV1VoteAccount(
                trans_data, 'validator_vote_account'
            )
        elif 'AddValidator' in l2_data.keys():
            output_buf += "AddValidator"
            output_buf += ValidateSolidoState("Add validators")
            trans_data = l2_data['AddValidator']
            output_buf += ValidateField(trans_data, 'solido_instance')
            output_buf += ValidateField(trans_data, 'manager')
            output_buf += ValidateAddV2VoteAccount(trans_data, 'validator_vote_account')
        elif 'MigrateStateToV2' in l2_data.keys():
            output_buf += ValidateTransOrder("MigrateStateToV2")
            output_buf += ValidateSolidoState("Upgrade program")
            trans_data = l2_data.get('MigrateStateToV2')
            output_buf += ValidateField(trans_data, 'solido_instance')
            output_buf += ValidateField(trans_data, 'manager')
            output_buf += ValidateField(trans_data, 'validator_list')
            output_buf += ValidateField(trans_data, 'maintainer_list')
            output_buf += ValidateField(trans_data, 'developer_account')
            output_buf += ValidateField(trans_data, 'max_maintainers')
            output_buf += ValidateField(trans_data, 'max_validators')
            output_buf += ValidateField(trans_data, 'max_commission_percentage')

            reward_distribution = trans_data.get('reward_distribution')
            output_buf += ValidateRewardField(reward_distribution, 'treasury_fee')
            output_buf += ValidateRewardField(reward_distribution, 'developer_fee')
            output_buf += ValidateRewardField(
                reward_distribution, 'st_sol_appreciation'
            )
        else:
            output_buf += "Unknown instruction\n"
            VerificationStatus = False
    elif 'BpfLoaderUpgrade' in l1_keys.keys():
        output_buf += ValidateTransOrder("BpfLoaderUpgrade")
        TransOrder.append("BpfLoaderUpgrade")
        output_buf += ValidateSolidoState("Upgrade program")
        l2_data = l1_keys['BpfLoaderUpgrade']
        output_buf += ValidateField(l2_data, 'program_to_upgrade')
        output_buf += ValidateField(l2_data, 'buffer_address')
    else:
        output_buf += "Unknown instruction\n"
        VerificationStatus = False

    print(output_buf)
    return VerificationStatus


def verify_transactions(ifile):
    Counter = 0
    Success = 0
    for transaction in ifile:
        result = solido(
            '--config',
            os.getenv("SOLIDO_CONFIG"),
            'multisig',
            'show-transaction',
            '--transaction-address',
            transaction.strip(),
        )
        Counter += 1
        print("Transaction #" + str(Counter) + ": " + transaction.strip())
        if verify_transaction_data(result):
            Success += 1
    print(
        "Summary: successfully verified "
        + str(Success)
        + " from "
        + str(Counter)
        + " transactions"
    )


if __name__ == '__main__':
    print("main")
