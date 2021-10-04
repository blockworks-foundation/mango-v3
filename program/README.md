## Tests

```
cargo test-bpf -- --show-output
```

## Deploy to devnet

```
cargo build-bpf && solana program deploy -k ~/.config/solana/devnet.json --program-id viQTKtBmaGvx3nugHcvijedy9ApbDowqiGYq35qAJqq ./target/deploy/mango.so
```

## Log Events
If you make changes to the log events defined in mango-logs/src/lib.rs, make sure to generate the IDL and copy it over
to mango-client-v3 for use in transaction logs scraper:
```
anchor build -p mango_logs
cp ~/blockworks-foundation/mango-v3/target/idl/mango_logs.json ~/blockworks-foundation/mango-client-v3/src/mango_logs.json
```
