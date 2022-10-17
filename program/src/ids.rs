pub mod srm_token {
    use solana_program::declare_id;
    #[cfg(feature = "devnet")]
    declare_id!("AvtB6w9xboLwA145E221vhof5TddhqsChYcx7Fy3xVMH");
    #[cfg(not(feature = "devnet"))]
    declare_id!("SRMuApVNdxXokk5GT7XD5cUUgXMBCoAz2LHeuAoKWRt");
}

pub mod msrm_token {
    use solana_program::declare_id;
    #[cfg(feature = "devnet")]
    declare_id!("8DJBo4bF4mHNxobjdax3BL9RMh5o71Jf8UiKsf5C5eVH");
    #[cfg(not(feature = "devnet"))]
    declare_id!("MSRMcoVyrFxnSgo5uXwone5SKcGhT1KEJMFEkMEWf9L");
}

pub mod mngo_token {
    use solana_program::declare_id;
    #[cfg(feature = "devnet")]
    declare_id!("Bb9bsTQa1bGEtQ5KagGkvSHyuLqDWumFUcRqFusFNJWC");
    #[cfg(not(feature = "devnet"))]
    declare_id!("MangoCzJ36AjZyKwVj3VnYU4GTonjfVEnJmvvWaxLac");
}

pub mod luna_pyth_oracle {
    use solana_program::declare_id;
    declare_id!("5bmWuR1dgP4avtGYMNKLuxumZTVKGgoN2BCMXWDNL9nY");
}

pub mod mainnet_1_group {
    use solana_program::declare_id;
    #[cfg(feature = "devnet")]
    declare_id!("Ec2enZyoC4nGpEfu2sUNAa2nUGJHWxoUWYSEJ2hNTWTA");
    #[cfg(not(feature = "devnet"))]
    declare_id!("98pjRuQjK3qA6gXts96PqZT4Ze5QmnCmt3QYjhbUSPue");
}

// Owner of the reimbursement fund multisig accounts
pub mod recovery_authority {
    use solana_program::declare_id;
    #[cfg(feature = "devnet")]
    declare_id!("8pANRWCcw8vn8DszUP7hh4xFbCiBiMWX3WbwUTipArSJ");
    #[cfg(not(feature = "devnet"))]
    declare_id!("9mM6NfXauEFviFY1S1thbo7HXYNiSWSvwZEhguJw26wY");
}
