# deploy program to testnet
if [ $# -eq 0 ]
  then
    KEYPAIR=~/.config/solana/devnet.json
  else
    KEYPAIR=$1
fi
CLUSTER_URL="https://api.testnet.solana.com"

# build and deploy mango
MANGO_PROGRAM_ID="BXhdkETgbHrr5QmVBT1xbz3JrMM28u5djbVtmTUfmFTH"
cargo build-bpf --features testnet --bpf-out-dir target/testnet
#solana program deploy target/testnet/mango.so --keypair $KEYPAIR --url $CLUSTER_URL --output json-compact
solana program deploy target/testnet/mango.so --keypair $KEYPAIR --url $CLUSTER_URL --program-id $MANGO_PROGRAM_ID --output json-compact

# build and deploy serum dex
# requires docker running
#cd ../serum-dex/dex
#SERUM_PROGRAM_ID="3qx9WcNPw4jj3v1kJbWoxSN2ZAakwUXFu9HDr2QjQ6xq"
#anchor build --verifiable
#solana program deploy target/verifiable/serum_dex.so --keypair $KEYPAIR --url $CLUSTER_URL --keypair $SERUM_PROGRAM_ID --output json-compact