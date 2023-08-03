//! Maintainer representation in the program state.

use std::fmt::Debug;

use serde::Serialize;

use borsh::{BorshDeserialize, BorshSchema, BorshSerialize};
use solana_program::{
    program_error::ProgramError,
    program_pack::Pack,
    program_pack::Sealed,
    pubkey::{Pubkey, PUBKEY_BYTES},
};

use crate::state::{AccountType, ListEntry};
use crate::util::serialize_b58;

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
