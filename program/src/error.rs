use bytemuck::Contiguous;
use solana_program::program_error::ProgramError;

use num_enum::IntoPrimitive;
use thiserror::Error;

pub type MerpsResult<T = ()> = Result<T, MerpsError>;

#[repr(u8)]
#[derive(Debug, Clone, Eq, PartialEq, Copy)]
pub enum SourceFileId {
    Processor = 0,
    State = 1,
}

impl std::fmt::Display for SourceFileId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceFileId::Processor => write!(f, "src/processor.rs"),
            SourceFileId::State => write!(f, "src/state.rs")
        }
    }
}


#[derive(Error, Debug, PartialEq, Eq)]
pub enum MerpsError {
    #[error(transparent)]
    ProgramError(#[from] ProgramError),
    #[error("{merps_error_code}; {source_file_id}:{line}")]
    MerpsErrorCode { merps_error_code: MerpsErrorCode, line: u32, source_file_id: SourceFileId },
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq, IntoPrimitive)]
#[repr(u32)]
pub enum MerpsErrorCode {
    #[error("MerpsErrorCode::InvalidOwner")]
    InvalidOwner,

    #[error("MerpsErrorCode::Default Check the source code for more info")]
    Default = u32::MAX_VALUE,
}

impl From<MerpsError> for ProgramError {
    fn from(e: MerpsError) -> ProgramError {
        match e {
            MerpsError::ProgramError(pe) => pe,
            MerpsError::MerpsErrorCode {
                merps_error_code,
                line: _,
                source_file_id: _
            } => ProgramError::Custom(merps_error_code.into()),
        }
    }
}

impl From<serum_dex::error::DexError> for MerpsError {
    fn from(de: serum_dex::error::DexError) -> Self {
        let pe: ProgramError = de.into();
        pe.into()
    }
}

#[inline]
pub fn check_assert(
    cond: bool,
    merps_error_code: MerpsErrorCode,
    line: u32,
    source_file_id: SourceFileId
) -> MerpsResult<()> {
    if cond {
        Ok(())
    } else {
        Err(MerpsError::MerpsErrorCode { merps_error_code, line, source_file_id })
    }
}
