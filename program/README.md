## Tests

```
cargo test-bpf -- --show-output
```

## Deploy to devnet

```
cargo build-bpf && solana program deploy -k ~/.config/solana/devnet.json --program-id viQTKtBmaGvx3nugHcvijedy9ApbDowqiGYq35qAJqq ./target/deploy/merps.so
```
