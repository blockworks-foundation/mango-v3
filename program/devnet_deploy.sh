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

cd ~/blockworks-foundation/merps/program

cargo build-bpf
solana program deploy target/deploy/merps.so --keypair $KEYPAIR --output json-compact

