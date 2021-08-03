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

cd ~/blockworks-foundation/mango-v3/program

mkdir target/devnet
cargo build-bpf --features devnet --bpf-out-dir target/devnet

#MANGO_PROGRAM_ID="EwG6vXKHmTPAS3K17CPu62AK3bdrrDJS3DibwUjv5ayT"
MANGO_PROGRAM_ID="5fP7Z7a87ZEVsKr2tQPApdtq83GcTW4kz919R6ou5h5E"
#MANGO_PROGRAM_ID="32WeJ46tuY6QEkgydqzHYU5j85UT9m1cPJwFxPjuSVCt"
solana program deploy target/devnet/mango.so --keypair $KEYPAIR --program-id $MANGO_PROGRAM_ID --output json-compact
#solana program deploy target/deploy/mango.so --keypair $KEYPAIR --output json-compact

# serum dex
VERSION=v1.6.18
sh -c "$(curl -sSfL https://release.solana.com/$VERSION/install)"

cd ~/blockworks-foundation/serum-dex/dex
cargo build-bpf --features devnet
DEX_PROGRAM_ID=DESVgJVGajEgKGXhb6XmqDHGz3VjdgP7rEVESBgxmroY
solana program deploy target/deploy/serum_dex.so --keypair $KEYPAIR --program-id $DEX_PROGRAM_ID

VERSION=v1.7.9
sh -c "$(curl -sSfL https://release.solana.com/$VERSION/install)"


### Example Mango Client CLI commands to launch a new group from source/cli.ts in mango-client-v3
###
### yarn cli init-group mango_test_v3.4 32WeJ46tuY6QEkgydqzHYU5j85UT9m1cPJwFxPjuSVCt DESVgJVGajEgKGXhb6XmqDHGz3VjdgP7rEVESBgxmroY EMjjdsqERN4wJUR9jMBax2pzqQPeGLNn5NeucbHpDUZK
### yarn cli add-oracle mango_test_v3.4 BTC
### yarn cli set-oracle mango_test_v3.4 BTC 40000000
### yarn cli add-spot-market mango_test_v3.4 BTC E1mfsnnCcL24JcDQxr7F2BpWjkyy5x2WHys8EL2pnCj9 bypQzRBaSDWiKhoAw3hNkf35eF3z3AZCU8Sxks6mTPP
### yarn cli add-perp-market mango_test_v3.4 BTC
