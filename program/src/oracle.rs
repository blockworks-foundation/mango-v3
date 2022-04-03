use arrayref::array_ref;
use fixed::types::I80F48;
use mango_common::Loadable;
use mango_macro::{Loadable, Pod};
use solana_program::{account_info::AccountInfo, pubkey::Pubkey, rent::Rent};
use std::{cell::RefMut, mem::size_of};

use crate::error::{check_assert, MangoErrorCode, MangoResult, SourceFileId};

declare_check_assert_macros!(SourceFileId::Oracle);

// oracle can be of different types
#[derive(PartialEq)]
#[repr(C)]
pub enum OracleType {
    Stub,
    Pyth,
    Switchboard,
    Unknown,
}

pub const STUB_MAGIC: u32 = 0x6F676E4D;

#[derive(Copy, Clone, Pod, Loadable)]
#[repr(C)]
pub struct StubOracle {
    pub magic: u32,    // Magic byte
    pub price: I80F48, // unit is interpreted as how many quote native tokens for 1 base native token
    pub last_update: u64,
}

// TODO move to separate program
impl StubOracle {
    pub fn load_mut_checked<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
    ) -> MangoResult<RefMut<'a, Self>> {
        check_eq!(account.data_len(), size_of::<Self>(), MangoErrorCode::Default)?;
        check_eq!(account.owner, program_id, MangoErrorCode::InvalidOwner)?;

        let oracle = Self::load_mut(account)?;

        Ok(oracle)
    }

    pub fn load_and_init<'a>(
        account: &'a AccountInfo,
        program_id: &Pubkey,
        rent: &Rent,
    ) -> MangoResult<RefMut<'a, Self>> {
        check_eq!(account.owner, program_id, MangoErrorCode::InvalidOwner)?;
        check!(
            rent.is_exempt(account.lamports(), account.data_len()),
            MangoErrorCode::AccountNotRentExempt
        )?;

        let oracle = Self::load_mut(account)?;

        Ok(oracle)
    }
}

pub fn determine_oracle_type(account: &AccountInfo) -> OracleType {
    let borrowed = account.data.borrow();
    let magic = u32::from_le_bytes(*array_ref![borrowed, 0, 4]);
    if magic == pyth_client::MAGIC {
        OracleType::Pyth
    } else if borrowed.len() == 1000 {
        OracleType::Switchboard
    } else if magic == STUB_MAGIC {
        OracleType::Stub
    } else {
        OracleType::Unknown
    }
}
