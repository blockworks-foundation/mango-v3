use bytemuck::Contiguous;
use solana_program::program_error::ProgramError;

use num_enum::IntoPrimitive;
use thiserror::Error;

pub type MangoResult<T = ()> = Result<T, MangoError>;

#[repr(u8)]
#[derive(Debug, Clone, Eq, PartialEq, Copy)]
pub enum SourceFileId {
    Processor = 0,
    State = 1,
    Critbit = 2,
    Queue = 3,
    Matching = 4,
    Oracle = 5,
}

impl std::fmt::Display for SourceFileId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceFileId::Processor => write!(f, "src/processor.rs"),
            SourceFileId::State => write!(f, "src/state.rs"),
            SourceFileId::Critbit => write!(f, "src/critbit"),
            SourceFileId::Queue => write!(f, "src/queue.rs"),
            SourceFileId::Matching => write!(f, "src/matching.rs"),
            SourceFileId::Oracle => write!(f, "src/oracle.rs"),
        }
    }
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum MangoError {
    #[error(transparent)]
    ProgramError(#[from] ProgramError),
    #[error("{mango_error_code}; {source_file_id}:{line}")]
    MangoErrorCode { mango_error_code: MangoErrorCode, line: u32, source_file_id: SourceFileId },
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq, IntoPrimitive)]
#[repr(u32)]
pub enum MangoErrorCode {
    #[error("MangoErrorCode::InvalidCache")]
    InvalidCache,
    #[error("MangoErrorCode::InvalidOwner")]
    InvalidOwner,
    #[error("MangoErrorCode::InvalidGroupOwner")]
    InvalidGroupOwner,
    #[error("MangoErrorCode::InvalidSignerKey")]
    InvalidSignerKey,
    #[error("MangoErrorCode::InvalidAdminKey")]
    InvalidAdminKey,
    #[error("MangoErrorCode::InvalidVault")]
    InvalidVault,
    #[error("MangoErrorCode::MathError")]
    MathError,
    #[error("MangoErrorCode::InsufficientFunds")]
    InsufficientFunds,
    #[error("MangoErrorCode::InvalidToken")]
    InvalidToken,
    #[error("MangoErrorCode::InvalidMarket")]
    InvalidMarket,
    #[error("MangoErrorCode::InvalidProgramId")]
    InvalidProgramId,
    #[error("MangoErrorCode::GroupNotRentExempt")]
    GroupNotRentExempt,
    #[error("MangoErrorCode::OutOfSpace")]
    OutOfSpace,
    #[error("MangoErrorCode::TooManyOpenOrders Reached the maximum number of open orders for this market")]
    TooManyOpenOrders,

    #[error("MangoErrorCode::AccountNotRentExempt")]
    AccountNotRentExempt,

    #[error("MangoErrorCode::ClientIdNotFound")]
    ClientIdNotFound,
    #[error("MangoErrorCode::InvalidNodeBank")]
    InvalidNodeBank,
    #[error("MangoErrorCode::InvalidRootBank")]
    InvalidRootBank,
    #[error("MangoErrorCode::MarginBasketFull")]
    MarginBasketFull,
    #[error("MangoErrorCode::NotLiquidatable")]
    NotLiquidatable,
    #[error("MangoErrorCode::Unimplemented")]
    Unimplemented,
    #[error("MangoErrorCode::PostOnly")]
    PostOnly,
    #[error("MangoErrorCode::Bankrupt Invalid instruction for bankrupt account")]
    Bankrupt,
    #[error("MangoErrorCode::InsufficientHealth")]
    InsufficientHealth,
    #[error("MangoErrorCode::InvalidParam")]
    InvalidParam,
    #[error("MangoErrorCode::InvalidAccount")]
    InvalidAccount,
    #[error("MangoErrorCode::InvalidAccountState")]
    InvalidAccountState,
    #[error("MangoErrorCode::SignerNecessary")]
    SignerNecessary,
    #[error("MangoErrorCode::InsufficientLiquidity Not enough deposits in this node bank")]
    InsufficientLiquidity,
    #[error("MangoErrorCode::InvalidOrderId")]
    InvalidOrderId,
    #[error("MangoErrorCode::InvalidOpenOrdersAccount")]
    InvalidOpenOrdersAccount,
    #[error("MangoErrorCode::BeingLiquidated Invalid instruction while being liquidated")]
    BeingLiquidated,
    #[error("MangoErrorCode::InvalidRootBankCache Cache the root bank to resolve")]
    InvalidRootBankCache,
    #[error("MangoErrorCode::InvalidPriceCache Cache the oracle price to resolve")]
    InvalidPriceCache,
    #[error("MangoErrorCode::InvalidPerpMarketCache Cache the perp market to resolve")]
    InvalidPerpMarketCache,
    #[error("MangoErrorCode::TriggerConditionFalse The trigger condition for this TriggerOrder is not met")]
    TriggerConditionFalse,
    #[error("MangoErrorCode::InvalidSeeds Invalid seeds. Unable to create PDA")]
    InvalidSeeds,
    #[error("MangoErrorCode::InvalidOracleType The oracle account was not recognized")]
    InvalidOracleType,
    #[error("MangoErrorCode::InvalidOraclePrice")]
    InvalidOraclePrice,
    #[error("MangoErrorCode::MaxAccountsReached The maximum number of accounts for this group has been reached")]
    MaxAccountsReached,

    #[error("MangoErrorCode::Default Check the source code for more info")]
    Default = u32::MAX_VALUE,
}

impl From<MangoError> for ProgramError {
    fn from(e: MangoError) -> ProgramError {
        match e {
            MangoError::ProgramError(pe) => pe,
            MangoError::MangoErrorCode { mango_error_code, line: _, source_file_id: _ } => {
                ProgramError::Custom(mango_error_code.into())
            }
        }
    }
}

impl From<serum_dex::error::DexError> for MangoError {
    fn from(de: serum_dex::error::DexError) -> Self {
        let pe: ProgramError = de.into();
        pe.into()
    }
}

#[inline]
pub fn check_assert(
    cond: bool,
    mango_error_code: MangoErrorCode,
    line: u32,
    source_file_id: SourceFileId,
) -> MangoResult<()> {
    if cond {
        Ok(())
    } else {
        Err(MangoError::MangoErrorCode { mango_error_code, line, source_file_id })
    }
}

#[macro_export]
macro_rules! declare_check_assert_macros {
    ($source_file_id:expr) => {
        #[allow(unused_macros)]
        macro_rules! check {
            ($cond:expr, $err:expr) => {
                check_assert($cond, $err, line!(), $source_file_id)
            };
        }

        #[allow(unused_macros)]
        macro_rules! check_eq {
            ($x:expr, $y:expr, $err:expr) => {
                check_assert($x == $y, $err, line!(), $source_file_id)
            };
        }

        #[allow(unused_macros)]
        macro_rules! throw {
            () => {
                MangoError::MangoErrorCode {
                    mango_error_code: MangoErrorCode::Default,
                    line: line!(),
                    source_file_id: $source_file_id,
                }
            };
        }

        #[allow(unused_macros)]
        macro_rules! throw_err {
            ($err:expr) => {
                MangoError::MangoErrorCode {
                    mango_error_code: $err,
                    line: line!(),
                    source_file_id: $source_file_id,
                }
            };
        }

        #[allow(unused_macros)]
        macro_rules! math_err {
            () => {
                MangoError::MangoErrorCode {
                    mango_error_code: MangoErrorCode::MathError,
                    line: line!(),
                    source_file_id: $source_file_id,
                }
            };
        }
    };
}
