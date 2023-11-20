#!/usr/bin/env python3

import argparse
import json
import sys
import os.path
from typing import Any

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.append(os.path.dirname(SCRIPT_DIR))

from testscripts.util import solido, solana, run  # type: ignore


def check_env(param):
    buf = param + " = " + os.getenv(param)
    if os.getenv(param) != None:
        buf += " [OK]"
    else:
        buf += " [BAD]"
    print(buf)


def verify_installation():
    check_env("PWD")
    check_env("SOLIDO_V2")
    check_env("SOLIDO_CONFIG")
    check_env("NETWORK")


def install_solido():
    pathStr = os.getenv("PWD")

    # install solido v2
    outout = os.system("cargo build --release")
    if os.path.isfile(pathStr + "/target/release/solido"):
        os.environ["SOLIDO_V2"] = pathStr + "/target/release/solido"
    else:
        print("Program not exist: " + pathStr + "/target/release/solido")
    output = os.chdir(pathStr)

    # install config
    os.environ["SOLIDO_CONFIG"] = pathStr + "/solido_config.json"
    os.environ["NETWORK"] = "https://api.mainnet-beta.solana.com"
    # verify installation
    verify_installation()
