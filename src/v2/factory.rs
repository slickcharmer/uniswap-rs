use super::{Library, Pair};
use crate::{
    bindings::i_uniswap_v2_factory::IUniswapV2Factory, errors::FactoryResult, ProtocolType,
};
use ethers::prelude::{builders::ContractCall, *};
use std::sync::Arc;

/// Represents a UniswapV2 factory.
#[derive(Clone, Debug)]
pub struct Factory<M> {
    /// The factory contract.
    contract: IUniswapV2Factory<M>,

    /// The factory protocol.
    protocol: ProtocolType,
}

impl<M> Factory<M> {
    /// Returns the contract address of the factory.
    pub fn address(&self) -> Address {
        self.contract.address()
    }

    /// Returns the protocol of the factory.
    pub fn protocol(&self) -> ProtocolType {
        self.protocol
    }

    /// Returns the deployment code's hash of the pair that this factory deploys.
    pub const fn pair_code_hash(&self) -> H256 {
        self.protocol.pair_code_hash()
    }
}

impl<M: Middleware> Factory<M> {
    /// Creates a new instance using the provided address.
    pub fn new(client: Arc<M>, address: Address, protocol: ProtocolType) -> Self {
        // assert!(protocol.is_v2(), "protocol must be v2");
        let contract = IUniswapV2Factory::new(address, client);
        Self { contract, protocol }
    }

    /// Creates a new instance using the provided chain.
    pub fn new_with_chain(client: Arc<M>, chain: Chain, protocol: ProtocolType) -> Option<Self> {
        // assert!(protocol.is_v2(), "protocol must be v2");
        protocol.try_addresses(chain).0.map(|address| {
            let contract = IUniswapV2Factory::new(address, client);
            Self { contract, protocol }
        })
    }

    /// Returns a reference to the factory contract.
    pub fn contract(&self) -> &IUniswapV2Factory<M> {
        &self.contract
    }

    /// Returns a reference to the client.
    pub fn client(&self) -> Arc<M> {
        // self.contract.client()
        todo!()
    }

    /// Returns the contract call for creating a pair.
    pub fn create_pair(&self, token_a: Address, token_b: Address) -> ContractCall<M, Address> {
        self.contract.create_pair(token_a, token_b)
    }

    /// Returns the pair for two token addresses.
    pub fn pair_for(&self, token_a: Address, token_b: Address) -> FactoryResult<Pair<M>, M> {
        let address = Library::pair_for(self, token_a, token_b)?;
        Ok(Pair::new(self.client(), address, self.protocol))
    }
}
