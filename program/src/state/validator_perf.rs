//! Validator performance metrics.

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

/// Each field is an optimum for a metric.
/// If a validator has a value for a metric that does not meet the threshold,
/// then the validator gets deactivated.
///
#[repr(C)]
#[derive(Clone, Debug, BorshDeserialize, BorshSerialize, BorshSchema, Eq, PartialEq, Serialize)]
pub struct Criteria {
    /// If a validator has the commission higher than this, then it gets deactivated.
    pub max_commission: u8,

    /// If a validator has `block_production_rate` lower than this, then it gets deactivated.
    pub min_block_production_rate: u64,

    /// If a validator has `vote_success_rate` lower than this, then it gets deactivated.
    pub min_vote_success_rate: u64,

    /// If a validator has the uptime lower than this, then it gets deactivated.
    pub min_uptime: u64,
}

impl Default for Criteria {
    fn default() -> Self {
        Self {
            max_commission: 100,
            min_vote_success_rate: 0,
            min_block_production_rate: 0,
            min_uptime: 0,
        }
    }
}

impl Criteria {
    pub fn new(
        max_commission: u8,
        min_vote_success_rate: u64,
        min_block_production_rate: u64,
        min_uptime: u64,
    ) -> Self {
        Self {
            max_commission,
            min_vote_success_rate,
            min_block_production_rate,
            min_uptime,
        }
    }
}

/// NOTE: ORDER IS VERY IMPORTANT HERE, PLEASE DO NOT RE-ORDER THE FIELDS UNLESS
/// THERE'S AN EXTREMELY GOOD REASON.
///
/// To save on BPF instructions, the serialized bytes are reinterpreted with an
/// unsafe pointer cast, which means that this structure cannot have any
/// undeclared alignment-padding in its representation.
#[repr(C)]
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, BorshSchema, Serialize)]
pub struct OffchainValidatorPerf {
    /// The epoch in which the off-chain part
    /// of the validator's performance was computed.
    pub updated_at: Epoch,

    /// The number of slots the validator has produced in the last epoch.
    pub block_production_rate: u64,

    /// Ratio of successful votes to total votes.
    pub vote_success_rate: u64,

    /// Ratio of how long the validator has been available to the total time in the epoch.
    pub uptime: u64,
}

/// NOTE: ORDER IS VERY IMPORTANT HERE, PLEASE DO NOT RE-ORDER THE FIELDS UNLESS
/// THERE'S AN EXTREMELY GOOD REASON.
///
/// To save on BPF instructions, the serialized bytes are reinterpreted with an
/// unsafe pointer cast, which means that this structure cannot have any
/// undeclared alignment-padding in its representation.
#[repr(C)]
#[derive(
    Clone, Debug, Default, Eq, PartialEq, BorshDeserialize, BorshSerialize, BorshSchema, Serialize,
)]
pub struct ValidatorPerf {
    /// The associated validator's vote account address.
    /// It might not be present in the validator list.
    /// Do not reorder this field, it should be first in the struct
    #[serde(serialize_with = "serialize_b58")]
    #[serde(rename = "pubkey")]
    pub validator_vote_account_address: Pubkey,

    /// The commission is updated at its own pace.
    pub commission: u8,
    pub commission_updated_at: Epoch,

    /// The off-chain part of the validator's performance, if available.
    pub rest: Option<OffchainValidatorPerf>,
}

impl ValidatorPerf {
    /// True only if these metrics meet the criteria.
    pub fn meets_criteria(&self, criteria: &Criteria) -> bool {
        self.commission <= criteria.max_commission
            && self.rest.as_ref().map_or(true, |perf| {
                perf.vote_success_rate >= criteria.min_vote_success_rate
                    && perf.block_production_rate >= criteria.min_block_production_rate
                    && perf.uptime >= criteria.min_uptime
            })
    }
}

impl ValidatorPerf {}

impl Sealed for ValidatorPerf {}

impl Pack for ValidatorPerf {
    const LEN: usize = 64;
    fn pack_into_slice(&self, data: &mut [u8]) {
        let mut data = data;
        BorshSerialize::serialize(&self, &mut data).unwrap();
    }
    fn unpack_from_slice(src: &[u8]) -> Result<Self, ProgramError> {
        let unpacked = Self::try_from_slice(src)?;
        Ok(unpacked)
    }
}

impl ListEntry for ValidatorPerf {
    const TYPE: AccountType = AccountType::ValidatorPerf;

    fn new(validator_vote_account_address: Pubkey) -> Self {
        Self {
            validator_vote_account_address,
            ..Default::default()
        }
    }

    fn pubkey(&self) -> &Pubkey {
        &self.validator_vote_account_address
    }
}
