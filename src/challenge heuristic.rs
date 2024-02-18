use web3::contract::{Contract, Options};
use web3::types::{Address, H256, U256};
use web3::Web3;
use web3::transports::Http;

use num_bigint::BigInt;
use sha3::{Digest, Keccak256};

#[tokio::main]
async fn main() -> web3::Result<()> {
    // Set up an HTTP transport layer.
    let http = Http::new("http://localhost:8545")?;
    let web3 = Web3::new(http);

    // Your contract's address here
    let contract_address = "YOUR_CONTRACT_ADDRESS_HERE";
    let address = Address::from_slice(&hex::decode(contract_address).unwrap());

    // Your contract's ABI here
    let contract_abi = include_str!("FiatShamirZKP.abi.json");

    // Create a contract instance
    let contract = Contract::from_json(
        web3.eth(),
        address,
        contract_abi.as_bytes(),
    )?;

    // Example of calling a constant function
    let result: U256 = contract.query("yourConstantFunction", (), None, Options::default(), None).await?;
    println!("Result: {}", result);

    // Example of sending a transaction to a non-constant function
    let tx = contract.call("yourNonConstantFunction", (param1, param2), "YOUR_WALLET_ADDRESS_HERE", Options::default()).await?;
    println!("Transaction Hash: {:?}", tx);

    Ok(())
}