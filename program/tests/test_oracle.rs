mod helpers;

use std::str::FromStr;
use fixed::types::I80F48;
use helpers::*;
use merps::{
    entrypoint::process_instruction,
    instruction::{
        add_spot_market, add_to_basket, cache_prices, cache_root_banks, deposit,
        init_merps_account, update_root_bank, withdraw, add_oracle, set_oracle
    },
    state::{MerpsAccount, MerpsGroup, NodeBank, QUOTE_INDEX},
};
use merps::oracle::{
    StubOracle,
    AccountType,
    Mapping,
    Product,
    Price,
    PriceType,
    PriceStatus,
    CorpAction,
    cast,
    MAGIC,
    VERSION_2,
    PROD_HDR_SIZE
};
use solana_program::account_info::AccountInfo;
use solana_program_test::*;
use solana_sdk::{
    account::Account,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use solana_client::{
  rpc_client::RpcClient
};
use std::mem::{size_of, size_of_val};

fn get_attr_str<'a,T>( ite: & mut T ) -> String
where T : Iterator<Item=& 'a u8>
{
  let mut len = *ite.next().unwrap() as usize;
  let mut val = String::with_capacity( len );
  while len > 0 {
    val.push( *ite.next().unwrap() as char );
    len -= 1;
  }
  return val
}

fn get_price_type( ptype: &PriceType ) -> &'static str
{
  match ptype {
    PriceType::Unknown    => "unknown",
    PriceType::Price      => "price",
  }
}

fn get_status( st: &PriceStatus ) -> &'static str
{
  match st {
    PriceStatus::Unknown => "unknown",
    PriceStatus::Trading => "trading",
    PriceStatus::Halted  => "halted",
    PriceStatus::Auction => "auction",
  }
}

fn get_corp_act( cact: &CorpAction ) -> &'static str
{
  match cact {
    CorpAction::NoCorpAct => "nocorpact",
  }
}


#[tokio::test]
async fn test_adapter() {
    let program_id = Pubkey::new_unique();
    let mut test = ProgramTest::new("merps", program_id, processor!(process_instruction));
    let oracle_pk = Pubkey::from_str("3m1y5h2uv7EQL3KaJZehvAJa4yDNvgc5yAdL9KPMKwvk").unwrap();
    let (mut banks_client, payer, recent_blockhash) = test.start().await;
    let res = tokio::task::spawn_blocking(|| {
        let url = "http://api.devnet.solana.com";
        let clnt = RpcClient::new( url.to_string() );
        let prod_pk = Pubkey::from_str("3m1y5h2uv7EQL3KaJZehvAJa4yDNvgc5yAdL9KPMKwvk").unwrap();
        let prod_data = clnt.get_account_data( &prod_pk ).unwrap();
        prod_data
    }).await.unwrap();
    let prod_pk = Pubkey::from_str("3m1y5h2uv7EQL3KaJZehvAJa4yDNvgc5yAdL9KPMKwvk").unwrap();
    // let account_info: AccountInfo = (&prod_pk, &mut res).into();
    // println!("=========== RES: {} ===========", account_info);
}

#[tokio::test]
async fn test_pyth() {
    // get pyth mapping account
    let blocking_task = tokio::task::spawn_blocking(|| {
        let url = "http://api.devnet.solana.com";
        let clnt = RpcClient::new( url.to_string() );
        let key = "BmA9Z6FjioHJPpjT39QazZyhDRUdZy2ezwx4GiDdE2u2";
        let mut akey = Pubkey::from_str( key ).unwrap();

        println!("Inside spawn_blocking");
        loop {
            // get Mapping account from key
            let map_data = clnt.get_account_data( &akey ).unwrap();
            let map_acct = cast::<Mapping>( &map_data );
            println!("MAGIC: {}", map_acct.magic);
            assert_eq!( map_acct.magic, MAGIC, "not a valid pyth account" );
            assert_eq!( map_acct.atype, AccountType::Mapping as u32,
                        "not a valid pyth mapping account" );
            assert_eq!( map_acct.ver, VERSION_2,
                        "unexpected pyth mapping account version" );

            // iget and print each Product in Mapping directory
            let mut i = 0;
            for prod_akey in &map_acct.products {
                let prod_pkey = Pubkey::new( &prod_akey.val );
                let prod_acc = clnt.get_account( &prod_pkey ).unwrap();
                let prod_data = clnt.get_account_data( &prod_pkey ).unwrap();
                println!("ACC LAMP: {}", &prod_acc.lamports);
                println!("ACC OWNER: {}", &prod_acc.owner.to_string());
                println!("ACC OWNER SIZE: {}", size_of_val(&prod_acc.owner));
                println!("ACC DATA SIZE: {}", size_of_val(&prod_acc.data));
                println!("SIZE ACC: {}", size_of_val( &prod_acc ));
                println!("SIZE: {}", size_of_val( &prod_data ));
                let prod_acct = cast::<Product>( &prod_data );
                println!("SIZE PROD: {}", size_of_val( prod_acct ));
                assert_eq!( prod_acct.magic, MAGIC, "not a valid pyth account" );
                assert_eq!( prod_acct.atype, AccountType::Product as u32,
                          "not a valid pyth product account" );
                assert_eq!( prod_acct.ver, VERSION_2,
                          "unexpected pyth product account version" );

                // print key and reference data for this Product
                println!( "product_account .. {:?}", prod_pkey );
                let mut psz = prod_acct.size as usize - PROD_HDR_SIZE;
                let mut pit = (&prod_acct.attr[..]).iter();
                while psz > 0 {
                    let key = get_attr_str( &mut pit );
                    let val = get_attr_str( &mut pit );
                    println!( "  {:.<16} {}", key, val );
                    psz -= 2 + key.len() + val.len();
                }

                // print all Prices that correspond to this Product
                if prod_acct.px_acc.is_valid() {
                    let mut px_pkey = Pubkey::new( &prod_acct.px_acc.val );
                    loop {
                        let pd = clnt.get_account_data( &px_pkey ).unwrap();
                        let pa = cast::<Price>( &pd );
                        assert_eq!( pa.magic, MAGIC, "not a valid pyth account" );
                        assert_eq!( pa.atype, AccountType::Price as u32,
                                 "not a valid pyth price account" );
                        assert_eq!( pa.ver, VERSION_2,
                                  "unexpected pyth price account version" );
                        println!( "  price_account .. {:?}", px_pkey );
                        println!( "    price_type ... {}", get_price_type(&pa.ptype));
                        println!( "    exponent ..... {}", pa.expo );
                        println!( "    status ....... {}", get_status(&pa.agg.status));
                        println!( "    corp_act ..... {}", get_corp_act(&pa.agg.corp_act));
                        println!( "    price ........ {}", pa.agg.price );
                        println!( "    conf ......... {}", pa.agg.conf );
                        println!( "    valid_slot ... {}", pa.valid_slot );
                        println!( "    publish_slot . {}", pa.agg.pub_slot );
                        println!( "    twap ......... {}", pa.twap );
                        println!( "    volatility ... {}", pa.avol );

                        // go to next price account in list
                        if pa.next.is_valid() {
                            px_pkey = Pubkey::new( &pa.next.val );
                        } else {
                            break;
                        }
                    }
                }
                // go to next product
                i += 1;
                if i == map_acct.num {
                    break;
                }
            }

            // go to next Mapping account in list
            if !map_acct.next.is_valid() {
              break;
            }
            akey = Pubkey::new( &map_acct.next.val );
        }
    });
    blocking_task.await.unwrap();
}

#[tokio::test]
async fn test_init_merps_group_with_oracle() {
    // Mostly a test to ensure we can successfully create the testing harness
    // Also gives us an alert if the InitMerpsGroup tx ends up using too much gas
    let program_id = Pubkey::new_unique();

    let mut test = ProgramTest::new("merps", program_id, processor!(process_instruction));

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(20_000);

    let merps_group = add_merps_group_prodlike(&mut test, program_id);
    let merps_group_pk = merps_group.merps_group_pk;

    assert_eq!(merps_group.num_oracles, 0);

    // let oracle_pk = add_test_account_with_owner::<StubOracle>(&mut test, &program_id);

    let oracle_pk = Pubkey::from_str("3m1y5h2uv7EQL3KaJZehvAJa4yDNvgc5yAdL9KPMKwvk").unwrap();

    let (mut banks_client, payer, recent_blockhash) = test.start().await;

    let mut transaction = Transaction::new_with_payer(
        &[
            merps_group.init_merps_group(&payer.pubkey()),
            add_oracle(&program_id, &merps_group_pk, &oracle_pk, &payer.pubkey()).unwrap(),
            // set_oracle(&program_id, &merps_group_pk, &oracle_pk, &payer.pubkey(), I80F48::from_num(40000)).unwrap(),
        ],
        Some(&payer.pubkey()),
    );

    println!(" ========== Test 1 ===========");

    transaction.sign(&[&payer], recent_blockhash);

    assert!(banks_client.process_transaction(transaction).await.is_ok());

    let mut account = banks_client.get_account(merps_group.merps_group_pk).await.unwrap().unwrap();
    let mut oracle_account = banks_client.get_account(oracle_pk).await.unwrap().unwrap();
    let account_info: AccountInfo = (&merps_group.merps_group_pk, &mut account).into();
    let oracle_ai: AccountInfo = (&oracle_pk, &mut oracle_account).into();
    let merps_group_loaded = MerpsGroup::load_mut_checked(&account_info, &program_id).unwrap();
    let mut oracle = StubOracle::load_mut_checked(&oracle_ai, &program_id).unwrap();
    println!("=============================");
    println!("Program ID: {}", program_id);
    println!("Merps group PK: {}", merps_group.merps_group_pk);
    println!("Oracle PK: {}", oracle_pk);
    println!("Oracle price: {}", oracle.price);
    oracle.price = I80F48::from_num(10000);
    println!("Oracle price after: {}", oracle.price);
    std::mem::drop(oracle);
    let mut oraclex2 = StubOracle::load_mut_checked(&oracle_ai, &program_id).unwrap();
    println!("Oracle price after x2: {}", oraclex2.price);
    println!("=============================");
    assert_eq!(merps_group_loaded.valid_interval, 5)
}

async fn test_oracle() {
    let program_id = Pubkey::new_unique();
    let user = Keypair::new();
    let admin = Keypair::new();
    let mut test = ProgramTest::new("merps", program_id, processor!(process_instruction));
    test.add_account(user.pubkey(), Account::new(u32::MAX as u64, 0, &user.pubkey()));

    let oracle_pk = add_test_account_with_owner::<StubOracle>(&mut test, &program_id);

    let quote_decimals = 6;
    let quote_unit = 10u64.pow(quote_decimals);

    let base_decimals = 6;
    let base_price = 40000;
    let base_unit = 10u64.pow(base_decimals);
    let oracle_price =
        I80F48::from_num(base_price) * I80F48::from_num(quote_unit) / I80F48::from_num(base_unit);
    println!("=============================");
    println!("{}", oracle_price);
    println!("=============================");

}
