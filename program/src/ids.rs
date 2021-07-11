pub mod srm_token {
    use solana_program::declare_id;
    #[cfg(feature = "devnet")]
    declare_id!("9FbAMDvXqNjPqZSYt4EWTguJuDrGkfvwr3gSFpiSbX9S");
    #[cfg(not(feature = "devnet"))]
    declare_id!("SRMuApVNdxXokk5GT7XD5cUUgXMBCoAz2LHeuAoKWRt");
}

pub mod msrm_token {
    use solana_program::declare_id;
    #[cfg(feature = "devnet")]
    declare_id!("934bNdNw9QfE8dXD4mKQiKajYURfSkPhxfYZzpvmygca");
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
