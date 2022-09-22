use super::{Factory, Library};
use crate::{bindings::i_uniswap_v2_pair::IUniswapV2Pair, errors::PairResult, ProtocolType};
use ethers::{abi::Token, contract::builders::ContractCall, core::abi::Detokenize, prelude::*};
use std::{fmt, sync::Arc};

type Tokens = (Address, Address);
type Reserves = (u128, u128, u32);

/// Represents a UniswapV2 liquidity pair, composed of 2 different ERC20 tokens.
#[derive(Clone)]
pub struct Pair<M> {
    /// The client.
    client: Arc<M>,

    /// The pair address. Might not be currently deployed.
    address: Address,

    /// The ordered tokens of the pair.
    tokens: Option<Tokens>,

    /// Whether the pair is currently deployed in the client's network.
    deployed: bool,

    /// The token reserves of the pair.
    reserves: Option<Reserves>,

    /// The protocol of the pair.
    protocol: ProtocolType,
}

// Skip client in formatting
impl<M> fmt::Debug for Pair<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pair")
            .field("address", &self.address)
            .field("tokens", &self.tokens)
            .field("deployed", &self.deployed)
            .field("reserves", &self.reserves)
            .finish()
    }
}

impl<M> fmt::Display for Pair<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.tokens.is_some() {
            writeln!(f, "Pair:     {:?}", self.address)?;
        } else {
            writeln!(f, "Pair: {:?}", self.address)?;
        }
        if let Some((a, b)) = self.tokens {
            writeln!(f, "Token0:   {:?}", a)?;
            write!(f, "Token1:   {:?}", b)?;
            if self.reserves.is_some() {
                writeln!(f)?
            };
        }
        if let Some((a, b, _)) = self.reserves {
            writeln!(f, "Reserve0: {:?}", a)?;
            write!(f, "Reserve1: {:?}", b)?;
        }
        Ok(())
    }
}

impl<M: Middleware> Pair<M> {
    /// Creates a new instance using the provided client and address.
    pub fn new(client: Arc<M>, address: Address, protocol: ProtocolType) -> Self {
        Self { client, address, tokens: None, deployed: false, reserves: None, protocol }
    }

    /// Creates a new instance using the provided client, factory and tokens' addresses.
    pub fn new_with_factory(
        client: Arc<M>,
        factory: Factory,
        token0: Address,
        token1: Address,
    ) -> PairResult<Self, M> {
        let (token0, token1) = Library::sort_tokens(token0, token1)?;
        let address = Library::pair_for(factory, token0, token1)?;

        Ok(Self {
            client,
            address,
            tokens: Some((token0, token1)),
            deployed: false,
            reserves: None,
            protocol: factory.protocol(),
        })
    }

    /// Returns the pair contract.
    pub fn contract(&self) -> IUniswapV2Pair<M> {
        IUniswapV2Pair::new(self.address, self.client.clone())
    }

    /// Returns the address of the pair.
    pub fn address(&self) -> Address {
        self.address
    }

    /// Returns whether the pair has been deployed.
    ///
    /// Note: this will always be false before syncing.
    pub fn deployed(&self) -> bool {
        self.deployed
    }

    /// Returns the addresses of the tokens that make up this pair.
    ///
    /// Note: this will always be None before syncing.
    pub fn tokens(&self) -> Option<Tokens> {
        self.tokens
    }

    /// Returns the reserves of the pair.
    ///
    /// Note: this will always be None before syncing.
    pub fn reserves(&self) -> Option<Reserves> {
        self.reserves
    }

    /// Returns the hash of the pair's deployment code. This can be used to determinalistically
    /// calculate the address of the pair given the addresses of 2 (sorted) tokens.
    pub const fn code_hash(&self) -> H256 {
        self.protocol.pair_code_hash()
    }

    /// Returns the contract calls for getting the addresses of the pair's tokens.
    pub fn get_tokens(&self) -> (ContractCall<M, Address>, ContractCall<M, Address>) {
        let pair = self.contract();
        (pair.token_0(), pair.token_1())
    }

    /// Returns the contract call for getting the reserves of the pair.
    pub fn get_reserves(&self) -> ContractCall<M, Reserves> {
        self.contract().get_reserves()
    }

    /// Syncs the tokens and reserves of the pair by querying the blockchain.
    ///
    /// Assumes that any call failure means the pair has not been deployed yet.
    pub async fn sync(
        &mut self,
        sync_tokens: bool,
        sync_reserves: bool,
    ) -> PairResult<&mut Self, M> {
        // let sync_tokens = sync_tokens || self.tokens.is_none() || !self.deployed;
        // let sync_reserves = sync_reserves || self.reserves.is_none();

        let multicall = Multicall::new(self.client.clone(), None).await?;
        let mut multicall = multicall.version(MulticallVersion::Multicall3);

        if sync_tokens {
            let calls = self.get_tokens();
            multicall.add_call(calls.0, true);
            multicall.add_call(calls.1, true);
        }

        if sync_reserves {
            multicall.add_call(self.get_reserves(), true);
        }

        let result = multicall.call_raw().await?;

        // Assume any call failure means the contract has not been deployed yet
        match (sync_tokens, sync_reserves) {
            (true, true) => {
                let tokens = parse_tokens_result(result[0..2].to_vec())?;
                let reserves = parse_reserves_result(result[2..].to_vec())?;

                if tokens.is_none() || reserves.is_none() {
                    self.tokens = None;
                    self.deployed = false;
                    return Ok(self)
                }

                self.deployed = true;
                self.tokens = tokens;
                self.reserves = reserves;
            }
            (true, false) => {
                let tokens = parse_tokens_result(result)?;

                if tokens.is_none() {
                    self.tokens = None;
                    self.deployed = false;
                    return Ok(self)
                }

                self.deployed = true;
                self.tokens = tokens;
            }
            (false, true) => {
                let reserves = parse_reserves_result(result)?;

                if reserves.is_none() {
                    self.tokens = None;
                    self.deployed = false;
                    return Ok(self)
                }

                self.deployed = true;
                self.reserves = reserves;
            }
            (false, false) => {}
        }

        Ok(self)
    }
}

/// Parses (bool, String) from a vector of tokens.
fn parse_errors(tokens: Vec<Token>) -> Vec<Option<String>> {
    type ErrorResult = (bool, String);
    let mut msgs = vec![];

    for token in tokens {
        match ErrorResult::from_tokens(vec![token]) {
            Ok((_, msg)) => msgs.push(Some(msg)),
            Err(_) => msgs.push(None),
        }
    }

    msgs
}

/// Parses a multicall result from a vector of tokens, returning None if the call returned an
/// error.
fn parse_result<M: Middleware, D: Detokenize>(tokens: Vec<Token>) -> PairResult<Option<D>, M> {
    let res = D::from_tokens(tokens.clone());
    match res {
        Err(e) => {
            // Failed to decode
            let errors = parse_errors(tokens);
            if errors.iter().any(|s| s.is_none()) {
                // Failed to decode errors too
                Err(ContractError::DetokenizationError(e).into())
            } else {
                // All calls failed while allowed
                Ok(None)
            }
        }
        Ok(res) => Ok(Some(res)),
    }
}

/// Parses a multicall result of Pair::get_tokens(), returning None if the call returned an
/// error.
fn parse_tokens_result<M: Middleware>(tokens: Vec<Token>) -> PairResult<Option<Tokens>, M> {
    type TokensResult = ((bool, Address), (bool, Address));
    let res: Option<TokensResult> = parse_result(tokens)?;

    match res {
        Some(res) => {
            if res.0 .0 && res.1 .0 {
                Ok(Some((res.0 .1, res.1 .1)))
            } else {
                Ok(None)
            }
        }
        None => Ok(None),
    }
}

/// Parses a multicall result of Pair::get_reserves(), returning None if the call returned an
/// error.
fn parse_reserves_result<M: Middleware>(tokens: Vec<Token>) -> PairResult<Option<Reserves>, M> {
    type ReservesResult = (bool, Reserves);
    let res: Option<ReservesResult> = parse_result(tokens)?;

    match res {
        Some(res) => {
            if res.0 {
                Ok(Some(res.1))
            } else {
                Ok(None)
            }
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{contracts::address, ProtocolType};

    fn default_pair() -> Pair<Provider<Http>> {
        let chain = Chain::Mainnet;
        let weth = address("WETH", chain);
        let usdc = address("USDC", chain);
        let provider = MAINNET.provider();
        let client = Arc::new(provider);
        let factory = Factory::new_with_chain(chain, ProtocolType::UniswapV2).unwrap();

        Pair::new_with_factory(client, factory, weth, usdc).unwrap()
    }

    #[test]
    fn test_parsing() {
        let addresses = (Address::random(), Address::random());
        // let tokens = vec![Token::Address(addresses.0), Token::Address(addresses.1)];
        let reserve_uints = (69u128, 420u128, 1337u32);
        let reserves = vec![
            Token::Uint(reserve_uints.0.into()),
            Token::Uint(reserve_uints.1.into()),
            Token::Uint(reserve_uints.2.into()),
        ];
        let error_message = "Error message".to_string();

        type SuccessResult = ((bool, Address), (bool, Address), (bool, Reserves));
        let success_result: SuccessResult =
            ((true, addresses.0), (true, addresses.1), (true, reserve_uints));
        let success_tokens = vec![
            Token::Tuple(vec![Token::Bool(true), Token::Address(addresses.0)]),
            Token::Tuple(vec![Token::Bool(true), Token::Address(addresses.1)]),
            Token::Tuple(vec![Token::Bool(true), Token::Tuple(reserves)]),
        ];
        type FailureResult = ((bool, String), (bool, String), (bool, String));
        let failure_result: FailureResult = (
            (false, error_message.clone()),
            (false, error_message.clone()),
            (false, error_message.clone()),
        );
        let failure_tokens = vec![
            Token::Tuple(vec![Token::Bool(false), Token::String(error_message.clone())]),
            Token::Tuple(vec![Token::Bool(false), Token::String(error_message.clone())]),
            Token::Tuple(vec![Token::Bool(false), Token::String(error_message.clone())]),
        ];

        // parse_errors

        let errors = parse_errors(success_tokens.clone());
        assert_eq!(errors.len(), 3);
        for e in errors {
            assert!(e.is_none());
        }

        let errors = parse_errors(failure_tokens.clone());
        assert_eq!(errors.len(), 3);
        for e in errors {
            assert_eq!(e.unwrap(), error_message.clone());
        }

        // parse_result

        let result = parse_result::<Provider<Http>, SuccessResult>(success_tokens.clone()).unwrap();
        assert_eq!(result.unwrap(), success_result);

        let result = parse_result::<Provider<Http>, FailureResult>(failure_tokens.clone()).unwrap();
        assert_eq!(result.unwrap(), failure_result);

        // parse_tokens_result

        let result = parse_tokens_result::<Provider<Http>>(success_tokens[0..2].to_vec()).unwrap();
        assert_eq!(result.unwrap(), addresses);

        let result = parse_tokens_result::<Provider<Http>>(failure_tokens.clone());
        assert!(result.unwrap().is_none());

        // parse_reserves_result

        let result = parse_reserves_result::<Provider<Http>>(success_tokens[2..].to_vec()).unwrap();
        assert_eq!(result.unwrap(), reserve_uints);

        let result = parse_reserves_result::<Provider<Http>>(failure_tokens);
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_sync() {
        let mut pair = default_pair();

        assert!(!pair.deployed());
        let tokens = pair.tokens().unwrap();
        assert_ne!(tokens.0, Address::zero());
        assert_ne!(tokens.1, Address::zero());

        pair.sync(true, false).await.unwrap();
        assert!(pair.deployed());
        assert!(pair.reserves().is_none());

        pair.sync(false, true).await.unwrap();
        let reserves = pair.reserves().unwrap();
        assert!(pair.deployed());
        assert_ne!(reserves.0, 0);
        assert_ne!(reserves.1, 0);
        assert_ne!(reserves.2, 0);
    }
}
