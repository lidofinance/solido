//! Maintainer representation in the program state.

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
#[derive(
    Clone, Default, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, BorshSchema, Serialize,
)]
pub struct Maintainer {
    /// Address of maintainer account.
    /// Do not reorder this field, it should be first in the struct
    #[serde(serialize_with = "serialize_b58")]
    pub pubkey: Pubkey,
}

impl Sealed for Maintainer {}

impl Pack for Maintainer {
    const LEN: usize = PUBKEY_BYTES;
    fn pack_into_slice(&self, data: &mut [u8]) {
        let mut data = data;
        BorshSerialize::serialize(&self, &mut data).unwrap();
    }
    fn unpack_from_slice(src: &[u8]) -> Result<Self, ProgramError> {
        let unpacked = Self::try_from_slice(src)?;
        Ok(unpacked)
    }
}

impl ListEntry for Maintainer {
    const TYPE: AccountType = AccountType::Maintainer;

    fn new(pubkey: Pubkey) -> Self {
        Self { pubkey }
    }

    fn pubkey(&self) -> &Pubkey {
        &self.pubkey
    }
}
