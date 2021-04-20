use std::cell::{Ref, RefMut};

use solana_program::account_info::AccountInfo;
use solana_program::program_error::ProgramError;
use bytemuck::{cast_slice, cast_slice_mut, from_bytes, from_bytes_mut, Pod, try_from_bytes, try_from_bytes_mut, Zeroable};

pub trait Loadable: Pod {
    fn load_mut<'a>(account: &'a AccountInfo) -> Result<RefMut<'a, Self>, ProgramError> {
        // TODO verify if this checks for size
        Ok(RefMut::map(account.try_borrow_mut_data()?, |data| from_bytes_mut(data)))
    }
    fn load<'a>(account: &'a AccountInfo) -> Result<Ref<'a, Self>, ProgramError> {
        Ok(Ref::map(account.try_borrow_data()?, |data| from_bytes(data)))
    }


    fn load_from_bytes(data: &[u8]) -> Result<&Self, ProgramError> {
        Ok(from_bytes(data))
    }
}

macro_rules! impl_loadable {
    ($type_name:ident) => {
        unsafe impl Zeroable for $type_name {}
        unsafe impl Pod for $type_name {}
        impl Loadable for $type_name {}
    }
}


pub const MAX_TOKENS: usize = 3;
#[derive(Copy, Clone)]
#[repr(C)]
pub struct MerpsGroup {
    pub last_updated: [u64; MAX_TOKENS]
}
impl_loadable!(MerpsGroup);