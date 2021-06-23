# devnet
if [ $# -eq 0 ]
  then
    KEYPAIR=~/.config/solana/devnet.json
  else
    KEYPAIR=$1
fi

# deploy mango program and new mango group
source ~/mango/cli/devnet.env $KEYPAIR
solana config set --url $CLUSTER_URL

cd ~/blockworks-foundation/mango/program

cargo build-bpf

MERPS_PROGRAM_ID="viQTKtBmaGvx3nugHcvijedy9ApbDowqiGYq35qAJqq"
solana program deploy target/deploy/mango.so --keypair $KEYPAIR --program-id $MERPS_PROGRAM_ID --output json-compact

# serum dex
VERSION=v1.6.9
sh -c "$(curl -sSfL https://release.solana.com/$VERSION/install)"

cd ~/blockworks-foundation/serum-dex/dex
cargo build-bpf --features devnet
DEX_PROGRAM_ID=DESVgJVGajEgKGXhb6XmqDHGz3VjdgP7rEVESBgxmroY
solana program deploy target/deploy/serum_dex.so --keypair $KEYPAIR --program-id $DEX_PROGRAM_ID

VERSION=v1.7.1
sh -c "$(curl -sSfL https://release.solana.com/$VERSION/install)"
