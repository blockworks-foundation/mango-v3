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

cd ~/blockworks-foundation/mango-v3/

mkdir target/devnet
cargo build-bpf --features devnet --bpf-out-dir target/devnet

# nightly
#MANGO_PROGRAM_ID="EwG6vXKHmTPAS3K17CPu62AK3bdrrDJS3DibwUjv5ayT"

# devnet.1
#MANGO_PROGRAM_ID="5fP7Z7a87ZEVsKr2tQPApdtq83GcTW4kz919R6ou5h5E"
# devnet.2
MANGO_PROGRAM_ID="4skJ85cdxQAFVKbcGgfun8iZPL7BadVYXG3kGEGkufqA"
solana program deploy target/devnet/mango.so --keypair $KEYPAIR --program-id $MANGO_PROGRAM_ID --output json-compact
#solana program deploy target/devnet/mango.so --keypair $KEYPAIR --output json-compact

# serum dex
DEX_PROGRAM_ID=DESVgJVGajEgKGXhb6XmqDHGz3VjdgP7rEVESBgxmroY
cd ~/blockworks-foundation/serum-dex/dex
anchor build --verifiable
solana program deploy target/verifiable/serum_dex.so --keypair $KEYPAIR --program-id $DEX_PROGRAM_ID

VERSION=v1.7.10
sh -c "$(curl -sSfL https://release.solana.com/$VERSION/install)"

### Example Mango Client CLI commands to launch a new group from source/cli.ts in mango-client-v3
###
### yarn cli init-group mango_test_v3.4 32WeJ46tuY6QEkgydqzHYU5j85UT9m1cPJwFxPjuSVCt DESVgJVGajEgKGXhb6XmqDHGz3VjdgP7rEVESBgxmroY EMjjdsqERN4wJUR9jMBax2pzqQPeGLNn5NeucbHpDUZK
### yarn cli add-oracle mango_test_v3.4 BTC
### yarn cli set-oracle mango_test_v3.4 BTC 40000000
### yarn cli add-spot-market mango_test_v3.4 BTC E1mfsnnCcL24JcDQxr7F2BpWjkyy5x2WHys8EL2pnCj9 bypQzRBaSDWiKhoAw3hNkf35eF3z3AZCU8Sxks6mTPP
### yarn cli add-perp-market mango_test_v3.4 BTC
