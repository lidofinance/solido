//! Types describing the state of the validator with respect to the pool.

use std::convert::TryFrom;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::ops::Range;

use serde::Serialize;

use borsh::{BorshDeserialize, BorshSchema, BorshSerialize};
use solana_program::{
    account_info::AccountInfo,
    borsh::{get_instance_packed_len, try_from_slice_unchecked},
    clock::Clock,
    clock::Epoch,
    entrypoint::ProgramResult,
    msg,
    program_error::ProgramError,
    program_memory::sol_memcmp,
    program_pack::Pack,
    program_pack::Sealed,
    pubkey::{Pubkey, PUBKEY_BYTES},
    rent::Rent,
    sysvar::Sysvar,
};
use spl_token::state::Mint;

use crate::big_vec::BigVec;
use crate::error::LidoError;
use crate::logic::{check_account_owner, get_reserve_available_balance};
use crate::metrics::Metrics;
use crate::processor::StakeType;
use crate::state::{AccountType, ListEntry, SeedRange};
use crate::token::{self, Lamports, Rational, StLamports};
use crate::util::serialize_b58;
use crate::{
    MINIMUM_STAKE_ACCOUNT_BALANCE, MINT_AUTHORITY, RESERVE_ACCOUNT, STAKE_AUTHORITY,
    VALIDATOR_STAKE_ACCOUNT, VALIDATOR_UNSTAKE_ACCOUNT,
};

/// NOTE: ORDER IS VERY IMPORTANT HERE, PLEASE DO NOT RE-ORDER THE FIELDS UNLESS
/// THERE'S AN EXTREMELY GOOD REASON.
///
/// To save on BPF instructions, the serialized bytes are reinterpreted with an
/// unsafe pointer cast, which means that this structure cannot have any
/// undeclared alignment-padding in its representation.
#[repr(C)]
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, BorshSchema, Serialize)]
pub struct Validator {
    /// Validator vote account address.
    /// Do not reorder this field, it should be first in the struct
    #[serde(serialize_with = "serialize_b58")]
    #[serde(rename = "pubkey")]
    pub vote_account_address: Pubkey,

    /// Seeds for active stake accounts.
    pub stake_seeds: SeedRange,
    /// Seeds for inactive stake accounts.
    pub unstake_seeds: SeedRange,

    /// Sum of the balances of the stake accounts and unstake accounts.
    pub stake_accounts_balance: Lamports,

    /// Sum of the balances of the unstake accounts.
    pub unstake_accounts_balance: Lamports,

    /// Effective stake balance is stake_accounts_balance - unstake_accounts_balance.
    /// The result is stored on-chain to optimize compute budget
    pub effective_stake_balance: Lamports,

    /// Controls if a validator is allowed to have new stake deposits.
    /// When removing a validator, this flag should be set to `false`.
    pub active: bool,
}

impl Validator {
    /// Return the balance in only the stake accounts, excluding the unstake accounts.
    pub fn compute_effective_stake_balance(&self) -> Lamports {
        (self.stake_accounts_balance - self.unstake_accounts_balance)
            .expect("Unstake balance cannot exceed the validator's total stake balance.")
    }

    pub fn observe_balance(observed: Lamports, tracked: Lamports, info: &str) -> ProgramResult {
        if observed < tracked {
            msg!(
                "{}: observed balance of {} is less than tracked balance of {}.",
                info,
                observed,
                tracked
            );
            msg!("This should not happen, aborting ...");
            return Err(LidoError::ValidatorBalanceDecreased.into());
        }
        Ok(())
    }

    pub fn has_stake_accounts(&self) -> bool {
        self.stake_seeds.begin != self.stake_seeds.end
    }
    pub fn has_unstake_accounts(&self) -> bool {
        self.unstake_seeds.begin != self.unstake_seeds.end
    }

    pub fn check_can_be_removed(&self) -> Result<(), LidoError> {
        if self.active {
            return Err(LidoError::ValidatorIsStillActive);
        }
        if self.has_stake_accounts() {
            return Err(LidoError::ValidatorShouldHaveNoStakeAccounts);
        }
        if self.has_unstake_accounts() {
            return Err(LidoError::ValidatorShouldHaveNoUnstakeAccounts);
        }
        // If not, this is a bug.
        assert_eq!(self.stake_accounts_balance, Lamports(0));
        Ok(())
    }

    pub fn show_removed_error_msg(error: &Result<(), LidoError>) {
        if let Err(err) = error {
            match err {
                LidoError::ValidatorIsStillActive => {
                    msg!(
                                "Refusing to remove validator because it is still active, deactivate it first."
                            );
                }
                LidoError::ValidatorHasUnclaimedCredit => {
                    msg!(
                        "Validator still has tokens to claim. Reclaim tokens before removing the validator"
                    );
                }
                LidoError::ValidatorShouldHaveNoStakeAccounts => {
                    msg!("Refusing to remove validator because it still has stake accounts, unstake them first.");
                }
                LidoError::ValidatorShouldHaveNoUnstakeAccounts => {
                    msg!("Refusing to remove validator because it still has unstake accounts, withdraw them first.");
                }
                _ => {
                    msg!("Invalid error when removing a validator: shouldn't happen.");
                }
            }
        }
    }

    pub fn find_stake_account_address_with_authority(
        &self,
        program_id: &Pubkey,
        solido_account: &Pubkey,
        authority: &[u8],
        seed: u64,
    ) -> (Pubkey, u8) {
        let seeds = [
            &solido_account.to_bytes(),
            &self.vote_account_address.to_bytes(),
            authority,
            &seed.to_le_bytes()[..],
        ];
        Pubkey::find_program_address(&seeds, program_id)
    }

    pub fn find_stake_account_address(
        &self,
        program_id: &Pubkey,
        solido_account: &Pubkey,
        seed: u64,
        stake_type: StakeType,
    ) -> (Pubkey, u8) {
        let authority = match stake_type {
            StakeType::Stake => VALIDATOR_STAKE_ACCOUNT,
            StakeType::Unstake => VALIDATOR_UNSTAKE_ACCOUNT,
        };
        self.find_stake_account_address_with_authority(program_id, solido_account, authority, seed)
    }

    /// Get stake account address that should be merged into another right after creation.
    /// This function should be used to create temporary stake accounts
    /// tied to the epoch that should be merged into another account and destroyed
    /// after a transaction. So that each epoch would have a different
    /// generation of stake accounts. This is done for security purpose
    pub fn find_temporary_stake_account_address(
        &self,
        program_id: &Pubkey,
        solido_account: &Pubkey,
        seed: u64,
        epoch: Epoch,
    ) -> (Pubkey, u8) {
        let authority = [VALIDATOR_STAKE_ACCOUNT, &epoch.to_le_bytes()[..]].concat();
        self.find_stake_account_address_with_authority(program_id, solido_account, &authority, seed)
    }

    /// Mark the validator as active so that they could receive new stake.
    pub fn activate(&mut self) {
        self.active = true;
    }

    /// Mark the validator as inactive so that no new stake can be delegated to it,
    /// and the existing stake shall be unstaked by the maintainer.
    pub fn deactivate(&mut self) {
        self.active = false;
    }
}

impl Sealed for Validator {}

impl Pack for Validator {
    const LEN: usize = 89;
    fn pack_into_slice(&self, data: &mut [u8]) {
        let mut data = data;
        BorshSerialize::serialize(&self, &mut data).unwrap();
    }
    fn unpack_from_slice(src: &[u8]) -> Result<Self, ProgramError> {
        let unpacked = Self::try_from_slice(src)?;
        Ok(unpacked)
    }
}

impl Default for Validator {
    fn default() -> Self {
        Validator {
            stake_seeds: SeedRange { begin: 0, end: 0 },
            unstake_seeds: SeedRange { begin: 0, end: 0 },
            stake_accounts_balance: Lamports(0),
            unstake_accounts_balance: Lamports(0),
            effective_stake_balance: Lamports(0),
            active: true,
            vote_account_address: Pubkey::default(),
        }
    }
}

impl ListEntry for Validator {
    const TYPE: AccountType = AccountType::Validator;

    fn new(vote_account_address: Pubkey) -> Self {
        Self {
            vote_account_address,
            ..Default::default()
        }
    }

    fn pubkey(&self) -> &Pubkey {
        &self.vote_account_address
    }
}
