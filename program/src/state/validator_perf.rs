//! Validator performance metrics.

use std::fmt::Debug;

use serde::Serialize;

use borsh::{BorshDeserialize, BorshSchema, BorshSerialize};
use solana_program::{
    program_error::ProgramError, program_pack::Pack, program_pack::Sealed, pubkey::Pubkey,
};

use crate::state::{AccountType, Epoch, ListEntry};
use crate::util::serialize_b58;

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
}

impl Default for Criteria {
    fn default() -> Self {
        Self {
            max_commission: 100,
            min_vote_success_rate: 0,
            min_block_production_rate: 0,
        }
    }
}

impl Criteria {
    pub fn new(
        max_commission: u8,
        min_vote_success_rate: u64,
        min_block_production_rate: u64,
    ) -> Self {
        Self {
            max_commission,
            min_vote_success_rate,
            min_block_production_rate,
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
