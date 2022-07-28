pub mod srm_token {
    use solana_program::declare_id;
    #[cfg(feature = "devnet")]
    declare_id!("AvtB6w9xboLwA145E221vhof5TddhqsChYcx7Fy3xVMH");
    #[cfg(feature = "testnet")]
    declare_id!("5vGS1gUhHcHCWNFnQGJ8uRaWVknfFPHS8fgvomkHx5fh");
    #[cfg(not(any(feature = "devnet", feature = "testnet")))]
    declare_id!("SRMuApVNdxXokk5GT7XD5cUUgXMBCoAz2LHeuAoKWRt");
}

pub mod msrm_token {
    use solana_program::declare_id;
    #[cfg(feature = "devnet")]
    declare_id!("8DJBo4bF4mHNxobjdax3BL9RMh5o71Jf8UiKsf5C5eVH");
    #[cfg(feature = "testnet")]
    declare_id!("3Ho7PN3bYv9bp1JDErBD2FxsRepPkL88vju3oDX9c3Ez");
    #[cfg(not(any(feature = "devnet", feature = "testnet")))]
    declare_id!("MSRMcoVyrFxnSgo5uXwone5SKcGhT1KEJMFEkMEWf9L");
}

pub mod mngo_token {
    use solana_program::declare_id;
    #[cfg(feature = "devnet")]
    declare_id!("Bb9bsTQa1bGEtQ5KagGkvSHyuLqDWumFUcRqFusFNJWC");
    #[cfg(feature = "testnet")]
    declare_id!("2hvukwp4UR9tqmCQhRzcsW9S2QBuU5Xcv5JJ5fUMmfvQ");
    #[cfg(not(any(feature = "devnet", feature = "testnet")))]
    declare_id!("MangoCzJ36AjZyKwVj3VnYU4GTonjfVEnJmvvWaxLac");
}

pub mod luna_pyth_oracle {
    use solana_program::declare_id;
    declare_id!("5bmWuR1dgP4avtGYMNKLuxumZTVKGgoN2BCMXWDNL9nY");
}
