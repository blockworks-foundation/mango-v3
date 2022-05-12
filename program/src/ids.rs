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

pub mod luna_spot_market {
    use solana_program::declare_id;
    declare_id!("HBTu8hNaoT3VyiSSzJYa8jwt9sDGKtJviSwFa11iXdmE");
}

pub mod luna_perp_market {
    use solana_program::declare_id;
    declare_id!("BCJrpvsB2BJtqiDgKVC4N6gyX1y24Jz96C6wMraYmXss");
}

pub mod luna_root_bank {
    use solana_program::declare_id;
    declare_id!("AUU8Zw5ezmZJBuWtMjfTTyP6eowkpNbH5pHh6uY5BHu7");
}
